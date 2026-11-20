//! fondos-animados-renderer: reproduce un vídeo en bucle, sin audio, en una ventana sin
//! decoración por monitor. Lo lanza la extensión de GNOME Shell, que esconde esas ventanas y
//! pinta su contenido (Clutter.Clone) dentro del fondo del escritorio y de Actividades.
//!
//! El vídeo se decodifica una sola vez (playbin3 → decodificador VA-API → gtk4paintablesink) y
//! el mismo GdkPaintable se muestra en todas las ventanas. Con VA-API los fotogramas viajan como
//! DMABuf hasta GTK y el compositor, sin pasar por la CPU.
//!
//! Si GTK lo permite (GtkGraphicsOffload con fondo negro, GTK >= 4.16), cada Picture va dentro de
//! un GraphicsOffload: el DMABuf del decodificador se entrega tal cual al compositor en una
//! subsuperficie, sin que GTK lo copie antes a su propio búfer en cada fotograma. Si no se puede
//! (GTK más viejo, --no-offload o fotogramas que no son DMABuf), se pinta como siempre: GTK
//! decide fotograma a fotograma y vuelve solo al camino normal cuando no puede descargarlo.
//!
//! Órdenes por stdin, una por línea:
//!   video <ruta>             cambia de vídeo
//!   fit cover|contain|fill   cómo se ajusta a la ventana
//!   pause | play             pausa / reanuda
//!   quit                     termina
//!
//! Opciones: --monitors AxB,..., --video <ruta>, --fit <modo>, --no-offload, --debug

use std::cell::{Cell, RefCell};
use std::io::BufRead;
use std::path::Path;
use std::rc::Rc;

use gst::prelude::*;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};

const TITULO_VENTANA: &str = "fondos-animados-renderer";

struct Opciones {
    monitores: Vec<(i32, i32)>,
    video: Option<String>,
    ajuste: gtk::ContentFit,
    offload: bool,
    depurar: bool,
}

fn ajuste_desde(texto: &str) -> gtk::ContentFit {
    match texto {
        "contain" => gtk::ContentFit::Contain,
        "fill" => gtk::ContentFit::Fill,
        _ => gtk::ContentFit::Cover,
    }
}

fn leer_opciones() -> Opciones {
    let mut op = Opciones {
        monitores: vec![],
        video: None,
        ajuste: gtk::ContentFit::Cover,
        offload: true,
        depurar: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--monitors" => {
                op.monitores = args
                    .next()
                    .unwrap_or_default()
                    .split(',')
                    .filter_map(|m| {
                        let (w, h) = m.split_once('x')?;
                        Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
                    })
                    .collect()
            }
            "--video" => op.video = args.next(),
            "--fit" => op.ajuste = ajuste_desde(&args.next().unwrap_or_default()),
            "--no-offload" => op.offload = false,
            "--debug" => op.depurar = true,
            "--version" => {
                println!("fondos-animados-renderer {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            otro => eprintln!("argumento desconocido: {otro}"),
        }
    }
    if op.monitores.is_empty() {
        op.monitores.push((1920, 1080));
    }
    op
}

struct Reproductor {
    playbin: gst::Element,
    ventanas: Vec<gtk::Window>,
    imagenes: Vec<gtk::Picture>,
    // El bucle se hace con seeks de segmento: al llegar al final del segmento (SEGMENT_DONE)
    // se vuelve al principio sin vaciar el pipeline, así no hay corte entre vuelta y vuelta.
    segmento_listo: Cell<bool>,
    pausado: Cell<bool>,
    depurar: bool,
}

impl Reproductor {
    fn poner_video(&self, ruta: &str) {
        let uri = match glib::filename_to_uri(Path::new(ruta), None) {
            Ok(uri) => uri,
            Err(e) => {
                eprintln!("ruta no válida {ruta}: {e}");
                return;
            }
        };
        let _ = self.playbin.set_state(gst::State::Ready);
        self.segmento_listo.set(false);
        self.playbin.set_property("uri", uri.as_str());
        let estado = if self.pausado.get() { gst::State::Paused } else { gst::State::Playing };
        if let Err(e) = self.playbin.set_state(estado) {
            eprintln!("no se pudo reproducir {ruta}: {e}");
        }
    }

    fn buscar_inicio(&self, vaciar: bool) {
        let mut flags = gst::SeekFlags::SEGMENT;
        if vaciar {
            flags |= gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE;
        }
        if let Err(e) = self.playbin.seek(
            1.0,
            flags,
            gst::SeekType::Set,
            gst::ClockTime::ZERO,
            gst::SeekType::None,
            gst::ClockTime::NONE,
        ) {
            eprintln!("seek fallido: {e}");
        }
    }

    fn pausar(&self, pausa: bool) {
        self.pausado.set(pausa);
        let estado = if pausa { gst::State::Paused } else { gst::State::Playing };
        let _ = self.playbin.set_state(estado);
    }

    fn mensaje(&self, msg: &gst::Message) {
        use gst::MessageView;
        match msg.view() {
            MessageView::AsyncDone(_) if !self.segmento_listo.get() => {
                self.segmento_listo.set(true);
                self.buscar_inicio(true);
                if self.depurar {
                    describir_pipeline(&self.playbin);
                }
            }
            MessageView::SegmentDone(_) => {
                if self.depurar {
                    eprintln!("fin de segmento: vuelta al principio");
                }
                self.buscar_inicio(false);
            }
            // Por si el demuxer no admite seeks de segmento.
            MessageView::Eos(_) => {
                self.segmento_listo.set(false);
                self.buscar_inicio(true);
            }
            MessageView::Error(err) => {
                eprintln!(
                    "error de GStreamer en {}: {} ({:?})",
                    err.src().map(|s| s.path_string()).unwrap_or_default(),
                    err.error(),
                    err.debug()
                );
            }
            MessageView::Warning(w) if self.depurar => eprintln!("aviso: {}", w.error()),
            _ => {}
        }
    }

    fn orden(&self, linea: &str) {
        let (orden, resto) = linea.split_once(' ').unwrap_or((linea, ""));
        match orden {
            "video" => self.poner_video(resto.trim()),
            "fit" => {
                let ajuste = ajuste_desde(resto.trim());
                for imagen in &self.imagenes {
                    imagen.set_content_fit(ajuste);
                }
            }
            "pause" => self.pausar(true),
            "play" => self.pausar(false),
            "quit" => {
                for v in &self.ventanas {
                    v.destroy();
                }
            }
            "" => {}
            otro => eprintln!("orden desconocida: {otro}"),
        }
    }
}

/// Con --debug: qué elementos ha montado playbin3 y qué caps llegan al sink. Si aparece un
/// decodificador va* y memory:DMABuf, la decodificación va por la GPU sin copias.
fn describir_pipeline(playbin: &gst::Element) {
    fn recorrer(bin: &gst::Bin, nivel: usize) {
        for el in bin.iterate_elements().into_iter().flatten() {
            let fabrica = el.factory().map(|f| f.name().to_string()).unwrap_or_default();
            eprintln!("{}{} ({})", "  ".repeat(nivel), el.name(), fabrica);
            if let Some(sub) = el.downcast_ref::<gst::Bin>() {
                recorrer(sub, nivel + 1);
            }
        }
    }
    if let Some(bin) = playbin.downcast_ref::<gst::Bin>() {
        recorrer(bin, 0);
    }
    let sink = playbin.property::<Option<gst::Element>>("video-sink");
    if let Some(caps) = sink.and_then(|s| s.static_pad("sink")).and_then(|p| p.current_caps()) {
        eprintln!("caps en el sink: {caps}");
    }
}

/// ¿Se puede usar GtkGraphicsOffload? Hace falta GTK >= 4.16 en tiempo de ejecución: la 4.14
/// ya lo tiene, pero sin «black-background» las bandas negras de «contain» las tendría que pintar
/// GTK debajo y la descarga casi nunca se daría. La propiedad se pone por nombre, así el binario
/// no depende de símbolos de la 4.16 y sigue arrancando con una GTK 4.14.
fn offload_disponible(op: &Opciones) -> bool {
    if !op.offload {
        return false;
    }
    if let Some(motivo) = gtk::check_version(4, 16, 0) {
        if op.depurar {
            eprintln!("sin GraphicsOffload: {motivo}");
        }
        return false;
    }
    // Desactivado a mano en GTK (GDK_DISABLE=offload o GDK_DEBUG=no-offload).
    let desactivado = |var: &str, clave: &str| {
        std::env::var(var).is_ok_and(|v| v.split([',', ' ']).any(|x| x == clave))
    };
    if desactivado("GDK_DISABLE", "offload") || desactivado("GDK_DEBUG", "no-offload") {
        if op.depurar {
            eprintln!("sin GraphicsOffload: desactivado por el entorno");
        }
        return false;
    }
    true
}

/// Los fotogramas de VA-API son NV12 BT.709, no sRGB: GTK solo los descarga si puede describir
/// su espacio de color al compositor (wp_color_manager_v1, que Mutter ofrece desde GNOME 48).
/// En GTK 4.24 eso sigue siendo opcional (GDK_DEBUG=color-mgmt); sin ello rechaza cada fotograma
/// («No color-management, non-default color state») y pinta por el camino clásico. Se activa
/// antes de gtk::init; si el compositor no lo ofrece, GTK sigue sin él y no pasa nada.
fn activar_gestion_color() {
    let actual = std::env::var("GDK_DEBUG").unwrap_or_default();
    if actual.split([',', ' ', ':']).any(|x| x == "color-mgmt") {
        return;
    }
    let nuevo = if actual.is_empty() { "color-mgmt".to_owned() } else { format!("{actual},color-mgmt") };
    // SAFETY: al principio de main, antes de crear ningún hilo.
    unsafe { std::env::set_var("GDK_DEBUG", nuevo) };
}

/// El widget que va en la ventana: la imagen sola o dentro de un GraphicsOffload.
fn envolver(imagen: &gtk::Picture, offload: bool) -> gtk::Widget {
    if !offload {
        return imagen.clone().upcast();
    }
    let envoltorio = gtk::GraphicsOffload::new(Some(imagen));
    envoltorio.set_property("black-background", true);
    envoltorio.upcast()
}

fn construir(app: &gtk::Application, op: &Rc<Opciones>) -> Option<Rc<Reproductor>> {
    let sink = match gst::ElementFactory::make("gtk4paintablesink").build() {
        Ok(s) => s,
        Err(_) => {
            eprintln!("falta gtk4paintablesink (paquete gstreamer1-plugin-gtk4)");
            app.quit();
            return None;
        }
    };
    let paintable = sink.property::<gdk::Paintable>("paintable");

    let playbin = gst::ElementFactory::make("playbin3")
        .property("video-sink", &sink)
        .build()
        .expect("playbin3");
    // Solo vídeo: sin audio, subtítulos, desentrelazado ni balance de color por software.
    playbin.set_property_from_str("flags", "video");

    let offload = offload_disponible(op);

    // Con GraphicsOffload, dentro de él solo puede ir el fotograma: un fondo CSS en la imagen
    // («Too much content») impide la descarga. El negro lo pone «black-background».
    let proveedor = gtk::CssProvider::new();
    proveedor.load_from_string(if offload {
        "window { background: black; } picture { background: none; }"
    } else {
        "window, picture { background: black; }"
    });
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display"),
        &proveedor,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    if op.depurar {
        eprintln!(
            "GTK {}.{}.{}: {}",
            gtk::major_version(),
            gtk::minor_version(),
            gtk::micro_version(),
            if offload { "con GraphicsOffload" } else { "sin GraphicsOffload (camino clásico)" }
        );
    }

    let mut ventanas = vec![];
    let mut imagenes = vec![];
    for (i, &(ancho, alto)) in op.monitores.iter().enumerate() {
        // Tamaño fijo, el del monitor. Una ventana no redimensionable toma el tamaño natural de
        // su contenido: al cambiar de vídeo (un instante sin imagen) se encogía y volvía a
        // crecer, y los clones de la extensión se quedaban con los bordes derecho e inferior
        // sin repintar. Redimensionable con su tamaño por defecto, y con ese mínimo, no se mueve.
        let imagen = gtk::Picture::builder()
            .paintable(&paintable)
            .content_fit(op.ajuste)
            .can_shrink(true)
            .width_request(ancho)
            .height_request(alto)
            .build();
        let ventana = gtk::ApplicationWindow::builder()
            .application(app)
            .title(format!("{TITULO_VENTANA}-{i}"))
            .decorated(false)
            .resizable(true)
            .default_width(ancho)
            .default_height(alto)
            .child(&envolver(&imagen, offload))
            .build();
        ventana.present();
        ventanas.push(ventana.upcast());
        imagenes.push(imagen);
    }

    let rep = Rc::new(Reproductor {
        playbin,
        ventanas,
        imagenes,
        segmento_listo: Cell::new(false),
        pausado: Cell::new(false),
        depurar: op.depurar,
    });

    let bus = rep.playbin.bus().expect("bus");
    let debil = Rc::downgrade(&rep);
    let vigilancia = bus
        .add_watch_local(move |_, msg| {
            if let Some(rep) = debil.upgrade() {
                rep.mensaje(msg);
            }
            glib::ControlFlow::Continue
        })
        .expect("bus watch");
    // La vigilancia del bus vive mientras viva la app.
    let vigilancia = RefCell::new(Some(vigilancia));
    app.connect_shutdown(move |_| drop(vigilancia.take()));

    if let Some(video) = &op.video {
        rep.poner_video(video);
    }
    Some(rep)
}

fn escuchar_stdin(rep: Rc<Reproductor>, app: gtk::Application) {
    let (tx, rx) = async_channel::unbounded::<String>();
    std::thread::spawn(move || {
        for linea in std::io::stdin().lock().lines() {
            let Ok(linea) = linea else { break };
            if tx.send_blocking(linea).is_err() {
                return;
            }
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(linea) = rx.recv().await {
            rep.orden(linea.trim());
        }
        // stdin cerrado (la extensión se ha ido): no seguimos solos.
        let _ = rep.playbin.set_state(gst::State::Null);
        app.quit();
    });
}

fn main() -> glib::ExitCode {
    glib::set_prgname(Some("fondos-animados-renderer"));
    glib::set_application_name("Fondos animados (renderizador)");
    let op = Rc::new(leer_opciones());
    if op.offload {
        activar_gestion_color();
    }
    // GTK antes que GStreamer: al conectarse, libwayland quita WAYLAND_SOCKET del entorno y
    // cierra el socket en exec. Si no, el gst-plugin-scanner que lanza gst::init al rehacer la
    // caché hereda nuestra conexión con mutter, algún plugin la usa y nos quedamos sin ventana
    // (y el scanner colgado para siempre).
    gtk::init().expect("gtk::init");
    gst::init().expect("gst::init");

    let app = gtk::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
    let reproductor: Rc<RefCell<Option<Rc<Reproductor>>>> = Rc::default();
    {
        let reproductor = reproductor.clone();
        app.connect_activate(move |app| {
            if reproductor.borrow().is_some() {
                return;
            }
            if let Some(rep) = construir(app, &op) {
                escuchar_stdin(rep.clone(), app.clone());
                reproductor.replace(Some(rep));
            }
        });
    }
    {
        let reproductor = reproductor.clone();
        app.connect_shutdown(move |_| {
            if let Some(rep) = reproductor.take() {
                let _ = rep.playbin.set_state(gst::State::Null);
            }
        });
    }
    // Sin argumentos para GApplication: los nuestros ya están leídos.
    app.run_with_args::<&str>(&[])
}
