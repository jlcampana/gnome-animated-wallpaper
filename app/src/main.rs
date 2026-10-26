//! Fondos animados: biblioteca de vídeos para la extensión de fondo animado de GNOME, con
//! búsqueda y descarga desde varias webs, conversión y navegador integrado.

mod actualizar;
mod bd;
mod biblioteca;
mod cola;
mod config;
mod contexto;
mod convertir;
mod descargas;
mod explorar;
mod fuentes;
mod gestor;
mod importar;
mod navegador;
mod pagina_ajustes;
mod previa;
mod red;

use std::path::PathBuf;

use adw::prelude::*;
use gtk::{gio, glib};

const CSS: &str = "
.marca-actual { background: @accent_bg_color; color: @accent_fg_color; border-radius: 999px; padding: 4px; }
.etiqueta-resolucion { background: rgba(0, 0, 0, 0.65); color: white; border-radius: 6px; padding: 1px 6px; }
/* Qué versión del fondo actual se está reproduciendo. */
.etiqueta-activa { background-color: @success_bg_color; color: @success_fg_color; font-weight: bold; border-radius: 6px; padding: 2px 8px; box-shadow: 0 1px 3px alpha(black, 0.45); }
/* Texto de la etiqueta de fps de la biblioteca (la forma roja se dibuja debajo). */
.etiqueta-fps { color: white; font-weight: bold; padding: 2px 8px 2px 15px; }
flowboxchild { border-radius: 14px; padding: 6px; }
flowboxchild.fondo-actual .miniatura { outline: 3px solid @accent_color; outline-offset: -3px; }
";

fn aplicar_estilo(ajustes: &gio::Settings) {
    let esquema = match ajustes.string("app-color-scheme").as_str() {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(esquema);
}

fn construir(app: &adw::Application) {
    let Some(ajustes) = config::ajustes() else {
        let v = adw::ApplicationWindow::builder().application(app).default_width(480).default_height(240).build();
        v.present();
        let d = adw::AlertDialog::new(
            Some("Falta la extensión"),
            Some("No está instalado el esquema de ajustes de la extensión «Fondos animados». Ejecuta install.sh del proyecto."),
        );
        d.add_response("cerrar", "Cerrar");
        let app = app.clone();
        d.connect_response(None, move |_, _| app.quit());
        d.present(Some(&v));
        return;
    };
    config::activar_depuracion(std::env::var_os("FONDOS_ANIMADOS_DEBUG").is_some() || ajustes.boolean("debug"));
    ajustes.connect_changed(Some("debug"), |a, _| {
        config::activar_depuracion(std::env::var_os("FONDOS_ANIMADOS_DEBUG").is_some() || a.boolean("debug"))
    });
    depurar!("arranque; biblioteca en {}", config::carpeta_biblioteca(&ajustes).display());
    aplicar_estilo(&ajustes);
    ajustes.connect_changed(Some("app-color-scheme"), |a, _| aplicar_estilo(a));

    let proveedor = gtk::CssProvider::new();
    proveedor.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().expect("display"),
        &proveedor,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let avisos = adw::ToastOverlay::new();
    let ventana = adw::ApplicationWindow::builder()
        .application(app)
        .title("Fondos animados")
        .default_width(1120)
        .default_height(780)
        .content(&avisos)
        .build();
    let ctx = contexto::Contexto::new(ajustes.clone(), ventana.clone(), avisos.clone());
    ctx.detectar_capacidades(false);

    let descargas = descargas::Descargas::new(ctx.clone());
    let biblioteca = biblioteca::Biblioteca::new(ctx.clone(), descargas.clone());
    let explorar = explorar::Explorar::new(ctx.clone(), descargas.clone());
    let navegador = navegador::Navegador::new(descargas.clone());
    let ajustes_pagina = pagina_ajustes::construir(&ctx);

    let pilas = adw::ViewStack::new();
    pilas.add_titled_with_icon(&biblioteca.raiz, Some("biblioteca"), "Biblioteca", "view-grid-symbolic");
    pilas.add_titled_with_icon(&explorar.raiz, Some("explorar"), "Explorar", "system-search-symbolic");
    pilas.add_titled_with_icon(&navegador.raiz, Some("navegador"), "Navegador", "web-browser-symbolic");
    pilas.add_titled_with_icon(&ajustes_pagina, Some("ajustes"), "Ajustes", "emblem-system-symbolic");
    {
        let explorar = explorar.clone();
        let navegador = navegador.clone();
        pilas.connect_visible_child_name_notify(move |p| match p.visible_child_name().as_deref() {
            Some("explorar") => explorar.cargar_si_vacia(),
            Some("navegador") => navegador.cargar_si_vacio(),
            _ => {}
        });
    }
    {
        let pilas = pilas.clone();
        let navegador = navegador.clone();
        ctx.abrir_navegador.replace(Some(Box::new(move |url: &str| {
            navegador.abrir(url);
            pilas.set_visible_child_name("navegador");
        })));
    }

    // Barra de título.
    let cabecera = adw::HeaderBar::new();
    cabecera.set_title_widget(Some(
        &adw::ViewSwitcher::builder().stack(&pilas).policy(adw::ViewSwitcherPolicy::Wide).build(),
    ));
    let menu_anadir = gio::Menu::new();
    menu_anadir.append(Some("Vídeos o paquetes (.zip, .rar, .7z)…"), Some("win.anadir"));
    menu_anadir.append(Some("Carpeta de Wallpaper Engine…"), Some("win.anadir-carpeta"));
    let anadir = gtk::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Añadir a la biblioteca")
        .menu_model(&menu_anadir)
        .build();
    cabecera.pack_start(&anadir);
    cabecera.pack_end(&menu_principal());
    cabecera.pack_end(&descargas.boton);

    let vista = adw::ToolbarView::new();
    vista.add_top_bar(&cabecera);
    let aviso_version = adw::Banner::new("");
    vista.add_top_bar(&aviso_version);
    actualizar::preparar(&ctx, &aviso_version);
    vista.set_content(Some(&pilas));
    avisos.set_child(Some(&vista));

    // Acciones.
    app.add_action(&ajustes.create_action("app-color-scheme"));
    let accion = |nombre: &str, f: Box<dyn Fn()>| {
        let a = gio::SimpleAction::new(nombre, None);
        a.connect_activate(move |_, _| f());
        ventana.add_action(&a);
    };
    {
        let pilas = pilas.clone();
        accion("ir-explorar", Box::new(move || pilas.set_visible_child_name("explorar")));
    }
    {
        let v = ventana.clone();
        let descargas = descargas.clone();
        accion(
            "anadir",
            Box::new(move || {
                let filtro = gtk::FileFilter::new();
                filtro.set_name(Some("Vídeos y paquetes"));
                for ext in config::EXTENSIONES_VIDEO.iter().chain(["zip", "rar", "7z"].iter()) {
                    filtro.add_suffix(ext);
                }
                let filtros = gio::ListStore::new::<gtk::FileFilter>();
                filtros.append(&filtro);
                let dialogo = gtk::FileDialog::builder().title("Añadir a la biblioteca").filters(&filtros).build();
                let descargas = descargas.clone();
                dialogo.open_multiple(Some(&v), gio::Cancellable::NONE, move |r| {
                    let Ok(lista) = r else { return };
                    let rutas: Vec<PathBuf> = (0..lista.n_items())
                        .filter_map(|i| lista.item(i).and_downcast::<gio::File>()?.path())
                        .collect();
                    descargas.importar_locales(rutas);
                });
            }),
        );
    }
    {
        let v = ventana.clone();
        let descargas = descargas.clone();
        accion(
            "anadir-carpeta",
            Box::new(move || {
                let dialogo = gtk::FileDialog::builder().title("Carpeta de un fondo de Wallpaper Engine").build();
                let descargas = descargas.clone();
                dialogo.select_folder(Some(&v), gio::Cancellable::NONE, move |r| {
                    if let Some(ruta) = r.ok().and_then(|f| f.path()) {
                        descargas.importar_locales(vec![ruta]);
                    }
                });
            }),
        );
    }
    {
        let biblioteca = biblioteca.clone();
        accion("sin-audio-todos", Box::new(move || biblioteca.quitar_audio_todos()));
    }
    {
        let biblioteca = biblioteca.clone();
        accion("versiones-30-todas", Box::new(move || biblioteca.crear_versiones_30_todas()));
    }
    {
        let biblioteca = biblioteca.clone();
        accion("rehacer-30-todas", Box::new(move || biblioteca.rehacer_versiones_30_todas()));
    }
    {
        let v = ventana.clone();
        accion(
            "acerca",
            Box::new(move || {
                let d = adw::AboutDialog::builder()
                    .application_name("Fondos animados")
                    .application_icon(config::ID_APP)
                    .version(env!("CARGO_PKG_VERSION"))
                    .developer_name("Jose Luis Campaña")
                    .website("https://github.com/jlcampana/gnome-animated-wallpaper")
                    .issue_url("https://github.com/jlcampana/gnome-animated-wallpaper/issues")
                    .license_type(gtk::License::MitX11)
                    .comments("Vídeos en bucle como fondo de escritorio de GNOME, decodificados por la GPU.")
                    .build();
                d.present(Some(&v));
            }),
        );
    }

    // Al cerrar con descargas del navegador en marcha, avisar (las de la cola las sigue el
    // gestor de descargas, aunque se cierre la ventana).
    {
        let descargas = descargas.clone();
        ventana.connect_close_request(move |v| {
            if !descargas.hay_activas() {
                return glib::Propagation::Proceed;
            }
            let d = adw::AlertDialog::new(
                Some("Hay descargas del navegador en curso"),
                Some("Si cierras ahora, se perderán. Las de la cola de descargas siguen aunque cierres."),
            );
            d.add_responses(&[("seguir", "Seguir"), ("cerrar", "Cerrar igualmente")]);
            d.set_response_appearance("cerrar", adw::ResponseAppearance::Destructive);
            let v2 = v.clone();
            d.connect_response(Some("cerrar"), move |_, _| v2.destroy());
            d.present(Some(v));
            glib::Propagation::Stop
        });
    }

    // Para depurar: FONDOS_ANIMADOS_PAGINA=explorar abre directamente esa pestaña.
    if let Ok(p) = std::env::var("FONDOS_ANIMADOS_PAGINA") {
        pilas.set_visible_child_name(&p);
    }
    ventana.present();
}

fn menu_principal() -> gtk::MenuButton {
    let estilo = gio::Menu::new();
    for (valor, texto) in [("auto", "Automático"), ("light", "Claro"), ("dark", "Oscuro")] {
        let item = gio::MenuItem::new(Some(texto), None);
        item.set_action_and_target_value(Some("app.app-color-scheme"), Some(&valor.to_variant()));
        estilo.append_item(&item);
    }
    let menu = gio::Menu::new();
    menu.append_submenu(Some("Estilo"), &estilo);
    let herramientas = gio::Menu::new();
    herramientas.append(Some("Quitar el audio de toda la biblioteca"), Some("win.sin-audio-todos"));
    herramientas.append(Some("Crear versiones de 30 fps de toda la biblioteca"), Some("win.versiones-30-todas"));
    herramientas.append(Some("Rehacer todas las versiones de 30 fps"), Some("win.rehacer-30-todas"));
    menu.append_section(None, &herramientas);
    let final_ = gio::Menu::new();
    final_.append(Some("Acerca de Fondos animados"), Some("win.acerca"));
    menu.append_section(None, &final_);
    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text("Menú principal")
        .menu_model(&menu)
        .primary(true)
        .build()
}

fn main() -> glib::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("fondos-animados {}", env!("CARGO_PKG_VERSION"));
        return glib::ExitCode::SUCCESS;
    }
    if std::env::args().nth(1).as_deref() == Some("--gestor-descargas") {
        return match gestor::ejecutar() {
            std::process::ExitCode::SUCCESS => glib::ExitCode::SUCCESS,
            _ => glib::ExitCode::FAILURE,
        };
    }
    if std::env::args().nth(1).as_deref() == Some("--probar-fuentes") {
        config::activar_depuracion(true);
        probar_fuentes();
        return glib::ExitCode::SUCCESS;
    }
    let app = adw::Application::builder().application_id(config::ID_APP).build();
    app.connect_activate(|app| {
        if let Some(v) = app.active_window() {
            v.present();
        } else {
            construir(app);
        }
    });
    app.run()
}

/// Diagnóstico: busca en cada web y enseña el primer resultado con su descarga.
fn probar_fuentes() {
    use fuentes::Fuente;
    for f in Fuente::TODAS.into_iter().filter(|f| f.nativa()) {
        match f.buscar("", 1, None, 1) {
            Ok(r) => {
                println!("{}: {} resultados", f.nombre(), r.len());
                if let Some(x) = r.first() {
                    println!("  {} ({:?})", x.titulo, x.resolucion);
                    match f.detalle(x) {
                        Ok(d) => {
                            println!("  previa: {:?}", d.vista_previa);
                            for des in d.descargas {
                                println!("  {}: {}", des.etiqueta, des.url);
                            }
                        }
                        Err(e) => println!("  ERROR detalle: {e}"),
                    }
                }
            }
            Err(e) => println!("{}: ERROR {e}", f.nombre()),
        }
    }
}
