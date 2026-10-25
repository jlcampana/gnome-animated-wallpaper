//! Página «Ajustes»: estado de la extensión, reproducción, descargas, hardware detectado y
//! estilo de la app, y la versión.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::config;
use crate::contexto::Contexto;

const ESTILOS: &[(&str, &str)] = &[("auto", "Automático (como el sistema)"), ("light", "Claro"), ("dark", "Oscuro")];

pub fn construir(ctx: &Rc<Contexto>) -> adw::PreferencesPage {
    let ajustes = &ctx.ajustes;
    let pagina = adw::PreferencesPage::new();

    // ── Extensión ──
    let grupo_ext = adw::PreferencesGroup::builder()
        .title("Extensión de GNOME Shell")
        .description("Es la que pone el vídeo en el fondo del escritorio")
        .build();
    let fila_ext = adw::ActionRow::builder().title("Fondos animados").subtitle("Comprobando…").build();
    let activar = gtk::Button::builder()
        .label("Activar")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .visible(false)
        .build();
    fila_ext.add_suffix(&activar);
    grupo_ext.add(&fila_ext);
    pagina.add(&grupo_ext);
    comprobar_extension(&fila_ext, &activar);
    {
        let fila = fila_ext.clone();
        activar.connect_clicked(move |b| {
            let _ = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).map(|bus| {
                bus.call_sync(
                    Some("org.gnome.Shell.Extensions"),
                    "/org/gnome/Shell/Extensions",
                    "org.gnome.Shell.Extensions",
                    "EnableExtension",
                    Some(&(config::UUID_EXTENSION,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    5000,
                    gio::Cancellable::NONE,
                )
            });
            comprobar_extension(&fila, b);
        });
    }

    // ── Reproducción ──
    let grupo = adw::PreferencesGroup::builder().title("Reproducción").build();
    let interruptor = |clave: &str, titulo: &str, subtitulo: &str| {
        let f = adw::SwitchRow::builder().title(titulo).subtitle(subtitulo).build();
        ajustes.bind(clave, &f, "active").build();
        grupo.add(&f);
    };
    interruptor("enabled", "Fondo animado activo", "");
    interruptor("pause-fullscreen", "Pausar con pantalla completa", "Cuando todas las pantallas tienen una ventana a pantalla completa");
    interruptor("pause-maximized", "Pausar con ventanas maximizadas", "");
    interruptor("pause-locked", "Pausar con la sesión bloqueada", "");
    let modos = ["cover", "contain", "fill"];
    let ajuste = adw::ComboRow::builder()
        .title("Ajuste a la pantalla")
        .model(&gtk::StringList::new(&["Recortar para llenar", "Entero, con bandas", "Estirar"]))
        .build();
    ajuste.set_selected(modos.iter().position(|m| *m == ajustes.string("fit")).unwrap_or(0) as u32);
    {
        let ajustes = ajustes.clone();
        ajuste.connect_selected_notify(move |c| {
            let _ = ajustes.set_string("fit", modos[c.selected() as usize]);
        });
    }
    grupo.add(&ajuste);
    let versiones = ["original", "30fps"];
    let version = adw::ComboRow::builder()
        .title("Versión que se usa de fondo")
        .subtitle("Cuando un vídeo tiene las dos: la de 30 fps hace trabajar menos a la GPU")
        .model(&gtk::StringList::new(&["La original", "La de 30 fps"]))
        .build();
    version.set_selected(versiones.iter().position(|v| *v == ajustes.string("preferred-version")).unwrap_or(0) as u32);
    {
        let ajustes = ajustes.clone();
        version.connect_selected_notify(move |c| {
            let _ = ajustes.set_string("preferred-version", versiones[c.selected() as usize]);
        });
    }
    grupo.add(&version);
    let bateria = adw::SwitchRow::builder()
        .title("Usar la de 30 fps con batería")
        .subtitle("Al desenchufar, el fondo pasa a su versión de 30 fps si la tiene; al enchufar, vuelve")
        .build();
    ajustes.bind("battery-30fps", &bateria, "active").build();
    grupo.add(&bateria);
    pagina.add(&grupo);

    // ── Descargas ──
    let grupo_desc = adw::PreferencesGroup::builder().title("Descargas e importación").build();
    let sin_audio = adw::SwitchRow::builder()
        .title("Quitar el audio")
        .subtitle("El fondo nunca suena: el audio solo ocupa disco. Se quita sin recodificar.")
        .build();
    ajustes.bind("strip-audio", &sin_audio, "active").build();
    grupo_desc.add(&sin_audio);
    let calidades = ["hd", "4k"];
    let calidad = adw::ComboRow::builder()
        .title("Calidad en MotionBGs")
        .model(&gtk::StringList::new(&["HD (1920×1080)", "4K (3840×2160)"]))
        .build();
    calidad.set_selected(calidades.iter().position(|c| *c == ajustes.string("download-quality")).unwrap_or(0) as u32);
    {
        let ajustes = ajustes.clone();
        calidad.connect_selected_notify(move |c| {
            let _ = ajustes.set_string("download-quality", calidades[c.selected() as usize]);
        });
    }
    grupo_desc.add(&calidad);
    let optimizar_opciones = ["auto", "always", "never"];
    let (w, h) = config::resolucion_pantalla();
    let optimizar = adw::ComboRow::builder()
        .title(format!("Adaptar a la pantalla ({w}×{h})"))
        .subtitle("Recodifica las descargas a tu resolución y al formato recomendado: ocupan menos y la GPU trabaja menos")
        .model(&gtk::StringList::new(&["Solo si la GPU puede codificar", "Siempre", "Nunca"]))
        .build();
    optimizar.set_selected(
        optimizar_opciones.iter().position(|c| *c == ajustes.string("optimize-downloads")).unwrap_or(0) as u32,
    );
    {
        let ajustes = ajustes.clone();
        optimizar.connect_selected_notify(move |c| {
            let _ = ajustes.set_string("optimize-downloads", optimizar_opciones[c.selected() as usize]);
        });
    }
    grupo_desc.add(&optimizar);
    let version_30 = adw::SwitchRow::builder()
        .title("Crear también la versión de 30 fps")
        .subtitle("De los vídeos que van más rápido, en la subcarpeta «30 fps»; el original se queda")
        .build();
    ajustes.bind("create-30fps", &version_30, "active").build();
    grupo_desc.add(&version_30);
    // Calidades medidas con x265 en un 1440p de 60 fps pasado a 30: 24 → 2,2 Mbps (VMAF 91),
    // 28 → 1,2 (87), 32 → 0,7 (81).
    let calidades = [24, 28, 32];
    let calidad_30 = adw::ComboRow::builder()
        .title("Calidad de las versiones de 30 fps")
        .subtitle("Para un fondo no hace falta tanta: menos calidad, menos disco y menos que decodificar")
        .model(&gtk::StringList::new(&["Alta", "Normal", "Ligera"]))
        .build();
    calidad_30.set_selected(calidades.iter().position(|c| *c == ajustes.int("quality-30fps")).unwrap_or(1) as u32);
    {
        let ajustes = ajustes.clone();
        calidad_30.connect_selected_notify(move |c| {
            let _ = ajustes.set_int("quality-30fps", calidades[c.selected() as usize]);
        });
    }
    grupo_desc.add(&calidad_30);
    let carpeta = adw::ActionRow::builder()
        .title("Carpeta de la biblioteca")
        .subtitle(ctx.biblioteca().to_string_lossy())
        .activatable(true)
        .build();
    carpeta.add_suffix(&gtk::Image::from_icon_name("folder-open-symbolic"));
    {
        let ctx = ctx.clone();
        carpeta.connect_activated(move |_| {
            let _ = std::fs::create_dir_all(ctx.biblioteca());
            let l = gtk::FileLauncher::new(Some(&gio::File::for_path(ctx.biblioteca())));
            l.launch(Some(&ctx.ventana), gio::Cancellable::NONE, |_| {});
        });
    }
    grupo_desc.add(&carpeta);
    pagina.add(&grupo_desc);

    // ── Hardware ──
    let grupo_hw = adw::PreferencesGroup::builder()
        .title("Hardware de vídeo de este equipo")
        .description("Se detecta al arrancar; la conversión usa lo mejor que haya")
        .build();
    let decod = adw::ActionRow::builder().title("Decodifica con la GPU").subtitle("Detectando…").build();
    let codif = adw::ActionRow::builder().title("Codifica con la GPU").subtitle("Detectando…").build();
    let recom = adw::ActionRow::builder().title("Formato recomendado").subtitle("Detectando…").build();
    let redetectar = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Volver a detectar")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    recom.add_suffix(&redetectar);
    for f in [&decod, &codif, &recom] {
        grupo_hw.add(f);
    }
    let filas = [decod, codif, recom];
    {
        let ctx2 = ctx.clone();
        let filas = filas.clone();
        redetectar.connect_clicked(move |_| {
            for f in &filas {
                f.set_subtitle("Detectando…");
            }
            ctx2.detectar_capacidades(true);
        });
    }
    ctx.conectar_capacidades(move |cap| {
        let lista = |v: &[String]| {
            if v.is_empty() {
                "Nada".to_string()
            } else {
                v.iter().map(|x| x.to_uppercase()).collect::<Vec<_>>().join(", ")
            }
        };
        filas[0].set_subtitle(&lista(&cap.decodifica_gpu));
        filas[1].set_subtitle(&if cap.codifica_gpu.is_empty() {
            "Nada: las conversiones van por la CPU".to_string()
        } else {
            cap.codifica_gpu.join(", ")
        });
        let donde = if cap.codifica_por_gpu() { "GPU" } else { "CPU" };
        filas[2].set_subtitle(&format!("{} con {} ({donde})", cap.recomendado.to_uppercase(), cap.codificador));
    });
    pagina.add(&grupo_hw);

    // ── Aspecto ──
    let grupo_estilo = adw::PreferencesGroup::builder().title("Aspecto de la app").build();
    let estilo = adw::ComboRow::builder()
        .title("Estilo")
        .model(&gtk::StringList::new(&ESTILOS.iter().map(|e| e.1).collect::<Vec<_>>()))
        .build();
    let sincronizar = {
        let estilo = estilo.downgrade();
        move |a: &gio::Settings| {
            if let Some(e) = estilo.upgrade() {
                let v = a.string("app-color-scheme");
                let i = ESTILOS.iter().position(|x| x.0 == v).unwrap_or(0) as u32;
                if e.selected() != i {
                    e.set_selected(i);
                }
            }
        }
    };
    sincronizar(ajustes);
    ajustes.connect_changed(Some("app-color-scheme"), move |a, _| sincronizar(a));
    {
        let ajustes = ajustes.clone();
        estilo.connect_selected_notify(move |c| {
            let _ = ajustes.set_string("app-color-scheme", ESTILOS[c.selected() as usize].0);
        });
    }
    grupo_estilo.add(&estilo);
    pagina.add(&grupo_estilo);

    let grupo_dep = adw::PreferencesGroup::builder().title("Depuración").build();
    let depurar = adw::SwitchRow::builder()
        .title("Registro de depuración")
        .subtitle("Detalle en el journal (journalctl --user -b | grep -i fondos) y en la salida de la app")
        .build();
    ajustes.bind("debug", &depurar, "active").build();
    grupo_dep.add(&depurar);
    pagina.add(&grupo_dep);

    let grupo_ver = adw::PreferencesGroup::new();
    let version = adw::ActionRow::builder().title("Versión").subtitle(env!("CARGO_PKG_VERSION")).build();
    version.add_css_class("property");
    grupo_ver.add(&version);
    pagina.add(&grupo_ver);

    pagina
}

fn comprobar_extension(fila: &adw::ActionRow, activar: &gtk::Button) {
    let fila = fila.clone();
    let activar = activar.clone();
    glib::spawn_future_local(async move {
        let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else { return };
        let r = bus
            .call_future(
                Some("org.gnome.Shell.Extensions"),
                "/org/gnome/Shell/Extensions",
                "org.gnome.Shell.Extensions",
                "GetExtensionInfo",
                Some(&(config::UUID_EXTENSION,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                5000,
            )
            .await;
        // state: 1 activa, 2 desactivada, 3 error, 4 desfasada, 6 inicializada…
        let estado = r
            .ok()
            .and_then(|v| glib::VariantDict::new(Some(&v.child_value(0))).lookup_value("state", None))
            .and_then(|s| s.get::<f64>());
        let (texto, boton) = match estado {
            None => ("No instalada, o hay que cerrar sesión y volver a entrar para que GNOME la vea", false),
            Some(s) if s == 1.0 => ("Activa", false),
            Some(s) if s == 2.0 || s == 6.0 => ("Instalada pero desactivada", true),
            Some(s) if s == 3.0 => ("Con errores (mira «journalctl --user -b | grep Fondos»)", false),
            Some(_) => ("Instalada; cierra sesión y vuelve a entrar para cargarla", false),
        };
        fila.set_subtitle(texto);
        activar.set_visible(boton);
    });
}
