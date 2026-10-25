//! Diálogo «Convertir»: resolución, ajuste, fps, tramo, fundido del bucle, códec y calidad.
//! Lanza fondo-animado-convertir --progreso y enseña el avance.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::contexto::Contexto;
use crate::{config, importar};

const AJUSTES: &[(&str, &str)] = &[
    ("cubrir", "Recortar para llenar"),
    ("encajar", "Entero, con bandas negras"),
    ("estirar", "Estirar"),
];
const FPS: &[&str] = &["original", "24", "30", "60"];
const CODECS: &[(&str, &str)] = &[
    ("auto", "Recomendado para este equipo"),
    ("hevc", "HEVC (H.265)"),
    ("h264", "H.264"),
    ("vp9", "VP9"),
    ("av1", "AV1"),
];

pub fn abrir(ctx: Rc<Contexto>, origen: &Path) {
    let (ancho, alto) = config::resolucion_pantalla();
    let resoluciones: Vec<String> = std::iter::once(format!("{ancho}x{alto}"))
        .chain(["1920x1080", "2560x1080", "2560x1440", "3440x1440", "3840x2160"].iter().map(|s| s.to_string()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut opciones_res = vec![format!("Pantalla ({ancho}×{alto})")];
    opciones_res.extend(resoluciones.iter().map(|r| r.replace('x', "×")));
    opciones_res.push("Original".into());

    let fila_info = adw::ActionRow::builder().title("Original").subtitle("Leyendo…").build();
    {
        let origen = origen.to_path_buf();
        let fila = fila_info.downgrade();
        glib::spawn_future_local(async move {
            let info = gio::spawn_blocking(move || importar::info(&origen)).await;
            if let (Ok(Ok(info)), Some(f)) = (info, fila.upgrade()) {
                f.set_subtitle(&info.resumen());
            }
        });
    }

    let lista = |v: Vec<String>| gtk::StringList::new(&v.iter().map(String::as_str).collect::<Vec<_>>());
    let resolucion = adw::ComboRow::builder().title("Resolución").model(&lista(opciones_res)).build();
    let ajuste = adw::ComboRow::builder()
        .title("Ajuste")
        .model(&lista(AJUSTES.iter().map(|a| a.1.to_string()).collect()))
        .build();
    let fps = adw::ComboRow::builder()
        .title("Fotogramas por segundo")
        .model(&lista(FPS.iter().map(|f| if *f == "original" { "Como el original".into() } else { f.to_string() }).collect()))
        .build();
    let inicio = adw::SpinRow::with_range(0.0, 36000.0, 0.5);
    inicio.set_title("Empezar en (s)");
    inicio.set_digits(1);
    let duracion = adw::SpinRow::with_range(0.0, 36000.0, 0.5);
    duracion.set_title("Duración (s)");
    duracion.set_subtitle("0 = hasta el final");
    duracion.set_digits(1);
    let fundido = adw::SpinRow::with_range(0.0, 5.0, 0.25);
    fundido.set_title("Fundido del bucle (s)");
    fundido.set_subtitle("Funde el final con el principio para que no se note el salto");
    fundido.set_digits(2);
    let codec = adw::ComboRow::builder()
        .title("Formato")
        .model(&lista(CODECS.iter().map(|c| c.1.to_string()).collect()))
        .build();
    {
        let codec = codec.downgrade();
        ctx.conectar_capacidades(move |cap| {
            if let Some(c) = codec.upgrade() {
                let como = if cap.codifica_por_gpu() { "con la GPU" } else { "con la CPU" };
                c.set_subtitle(&format!("Recomendado: {} {como} ({})", cap.recomendado.to_uppercase(), cap.codificador));
            }
        });
    }
    let cpu = adw::SwitchRow::builder()
        .title("Forzar la CPU")
        .subtitle("Más lento, a veces algo más de calidad")
        .build();
    let calidad = adw::SpinRow::with_range(14.0, 40.0, 1.0);
    calidad.set_title("Calidad");
    calidad.set_subtitle("Menos es mejor calidad y más peso");
    calidad.set_value(24.0);

    let base = origen.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let nombre = adw::EntryRow::builder().title("Nombre").text(format!("{base}-{ancho}x{alto}")).build();
    let reemplazar = adw::SwitchRow::builder()
        .title("Sustituir el original")
        .subtitle("Si no, se guarda como un vídeo nuevo")
        .build();
    let usar = adw::SwitchRow::builder().title("Usar de fondo al terminar").active(true).build();

    let grupo_origen = adw::PreferencesGroup::new();
    grupo_origen.add(&fila_info);
    let grupo_imagen = adw::PreferencesGroup::builder().title("Imagen").build();
    for f in [resolucion.upcast_ref::<gtk::Widget>(), ajuste.upcast_ref(), fps.upcast_ref()] {
        grupo_imagen.add(f);
    }
    let grupo_tramo = adw::PreferencesGroup::builder().title("Tramo y bucle").build();
    for f in [inicio.upcast_ref::<gtk::Widget>(), duracion.upcast_ref(), fundido.upcast_ref()] {
        grupo_tramo.add(f);
    }
    let grupo_codec = adw::PreferencesGroup::builder().title("Codificación").build();
    for f in [codec.upcast_ref::<gtk::Widget>(), cpu.upcast_ref(), calidad.upcast_ref()] {
        grupo_codec.add(f);
    }
    let grupo_salida = adw::PreferencesGroup::builder().title("Resultado").build();
    for f in [nombre.upcast_ref::<gtk::Widget>(), reemplazar.upcast_ref(), usar.upcast_ref()] {
        grupo_salida.add(f);
    }

    let pagina = adw::PreferencesPage::new();
    for g in [&grupo_origen, &grupo_imagen, &grupo_tramo, &grupo_codec, &grupo_salida] {
        pagina.add(g);
    }

    let barra = gtk::ProgressBar::builder().show_text(true).visible(false).build();
    let estado = gtk::Label::builder().css_classes(["dim-label", "caption"]).visible(false).wrap(true).build();
    let progreso = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_start(12)
        .margin_end(12)
        .margin_bottom(12)
        .build();
    progreso.append(&barra);
    progreso.append(&estado);

    let boton = gtk::Button::builder().label("Convertir").css_classes(["suggested-action"]).build();
    let cabecera = adw::HeaderBar::new();
    cabecera.pack_end(&boton);

    let vista = adw::ToolbarView::new();
    vista.add_top_bar(&cabecera);
    vista.set_content(Some(&pagina));
    vista.add_bottom_bar(&progreso);

    let dialogo = adw::Dialog::builder()
        .title(format!("Convertir «{base}»"))
        .content_width(560)
        .content_height(720)
        .child(&vista)
        .build();

    let proceso: Rc<RefCell<Option<gio::Subprocess>>> = Rc::default();
    {
        let proceso = proceso.clone();
        dialogo.connect_closed(move |_| {
            if let Some(p) = proceso.take() {
                p.send_signal(15);
            }
        });
    }

    let ventana = ctx.ventana.clone();
    let origen = origen.to_path_buf();
    let d = dialogo.downgrade();
    boton.connect_clicked(move |boton| {
        if let Some(p) = proceso.take() {
            p.send_signal(15);
            return;
        }
        let res = match resolucion.selected() as usize {
            0 => format!("{ancho}x{alto}"),
            i if i <= resoluciones.len() => resoluciones[i - 1].clone(),
            _ => "original".into(),
        };
        let biblioteca = ctx.biblioteca();
        let _ = std::fs::create_dir_all(&biblioteca);
        let limpio = config::nombre_limpio(&nombre.text());
        let destino = if reemplazar.is_active() {
            origen.with_extension("convirtiendo.mp4")
        } else {
            config::ruta_libre(&biblioteca, &limpio, "mp4")
        };
        let mut args: Vec<String> = vec![
            config::programa("fondo-animado-convertir").to_string_lossy().into_owned(),
            "--progreso".into(),
            "-r".into(),
            res,
            "-a".into(),
            AJUSTES[ajuste.selected() as usize].0.into(),
            "-f".into(),
            FPS[fps.selected() as usize].into(),
            "-i".into(),
            inicio.value().to_string(),
            "-d".into(),
            duracion.value().to_string(),
            "-b".into(),
            fundido.value().to_string(),
            "-c".into(),
            CODECS[codec.selected() as usize].0.into(),
            "-q".into(),
            (calidad.value() as i32).to_string(),
        ];
        if cpu.is_active() {
            args.push("--cpu".into());
        }
        args.push(origen.to_string_lossy().into_owned());
        args.push(destino.to_string_lossy().into_owned());

        crate::depurar!("convertir: {}", args.join(" "));
        let argv: Vec<&std::ffi::OsStr> = args.iter().map(|a| std::ffi::OsStr::new(a.as_str())).collect();
        let sp = match gio::Subprocess::newv(&argv, gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE) {
            Ok(sp) => sp,
            Err(e) => {
                ctx.aviso(&format!("No se pudo lanzar la conversión: {e}"));
                return;
            }
        };
        proceso.replace(Some(sp.clone()));
        pagina.set_sensitive(false);
        boton.set_label("Cancelar");
        boton.remove_css_class("suggested-action");
        boton.add_css_class("destructive-action");
        barra.set_visible(true);
        barra.set_fraction(0.0);
        barra.set_text(Some("Empezando…"));
        estado.set_visible(true);
        estado.set_label("");

        let ctx = ctx.clone();
        let barra = barra.clone();
        let estado = estado.clone();
        let boton = boton.clone();
        let pagina = pagina.clone();
        let proceso = proceso.clone();
        let origen = origen.clone();
        let reemplazar = reemplazar.is_active();
        let usar = usar.is_active();
        let d = d.clone();
        glib::spawn_future_local(async move {
            let lector = gio::DataInputStream::new(&sp.stdout_pipe().expect("stdout"));
            let mut duracion = 0.0f64;
            while let Ok(Some(linea)) = lector.read_line_utf8_future(glib::Priority::DEFAULT).await {
                let linea = linea.trim();
                if let Some(d) = linea.strip_prefix("DURACION ") {
                    duracion = d.parse().unwrap_or(0.0);
                } else if let Some(c) = linea.strip_prefix("CODIFICADOR ") {
                    let donde = if c.starts_with("lib") { "CPU" } else { "GPU" };
                    estado.set_label(&format!("Codificando con {c} ({donde})"));
                } else if linea == "REINICIO" {
                    estado.set_label("El codificador ha fallado; pruebo otro…");
                } else if let Some(us) = linea.strip_prefix("out_time_us=") {
                    if let (Ok(us), true) = (us.parse::<f64>(), duracion > 0.0) {
                        let f = (us / 1e6 / duracion).clamp(0.0, 1.0);
                        barra.set_fraction(f);
                        barra.set_text(Some(&format!("{:.0} %", f * 100.0)));
                    }
                } else if let Some(v) = linea.strip_prefix("speed=") {
                    barra.set_tooltip_text(Some(&format!("Velocidad: {v}")));
                }
            }
            let ok = sp.wait_check_future().await.is_ok();
            let cancelado = proceso.take().is_none();
            pagina.set_sensitive(true);
            boton.set_label("Convertir");
            boton.remove_css_class("destructive-action");
            boton.add_css_class("suggested-action");
            if cancelado {
                barra.set_visible(false);
                estado.set_label("Cancelada");
                let _ = std::fs::remove_file(&destino);
                return;
            }
            if !ok {
                let mut error = String::new();
                if let Some(e) = sp.stderr_pipe() {
                    let lector = gio::DataInputStream::new(&e);
                    while let Ok(Some(l)) = lector.read_line_utf8_future(glib::Priority::DEFAULT).await {
                        error = l.to_string();
                    }
                }
                estado.set_label(&format!("Error: {error}"));
                barra.set_visible(false);
                return;
            }
            let final_ = if reemplazar {
                let f = origen.with_extension("mp4");
                let _ = std::fs::remove_file(&origen);
                let _ = std::fs::rename(&destino, &f);
                f
            } else {
                destino
            };
            ctx.biblioteca_cambiada();
            if usar {
                ctx.usar_video(&final_);
            }
            ctx.aviso(&format!(
                "Conversión terminada: {}",
                final_.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ));
            if let Some(d) = d.upgrade() {
                d.close();
            }
        });
    });

    dialogo.present(Some(&ventana));
}
