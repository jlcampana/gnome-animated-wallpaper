//! Página «Navegador»: WebKit con sesión persistente (las cookies de los captchas se guardan).
//! Toda descarga (vídeos, .zip de Wallpaper Engine…) se captura y se importa a la biblioteca.
//! Las ventanas emergentes (publicidad) se bloquean.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use webkit6::prelude::*;

use crate::config;
use crate::descargas::Descargas;
use crate::fuentes::Fuente;

pub struct Navegador {
    pub raiz: gtk::Box,
    pub vista: webkit6::WebView,
}

impl Navegador {
    pub fn new(descargas: Rc<Descargas>) -> Rc<Self> {
        let datos = config::carpeta_datos().join("web");
        let cache = config::carpeta_cache().join("web");
        let sesion = webkit6::NetworkSession::new(Some(&datos.to_string_lossy()), Some(&cache.to_string_lossy()));
        if let Some(cookies) = sesion.cookie_manager() {
            cookies.set_persistent_storage(
                &datos.join("cookies.sqlite").to_string_lossy(),
                webkit6::CookiePersistentStorage::Sqlite,
            );
        }
        sesion.connect_download_started(move |_, descarga| descargas.seguir_webkit(descarga));

        let vista = webkit6::WebView::builder().network_session(&sesion).vexpand(true).build();
        if let Some(ajustes) = webkit6::prelude::WebViewExt::settings(&vista) {
            ajustes.set_javascript_can_open_windows_automatically(false);
            ajustes.set_media_playback_requires_user_gesture(true);
        }

        // Lo que WebKit no sabe mostrar, y los vídeos sueltos, se descargan.
        vista.connect_decide_policy(|_, decision, tipo| {
            if tipo != webkit6::PolicyDecisionType::Response {
                return false;
            }
            let Some(d) = decision.downcast_ref::<webkit6::ResponsePolicyDecision>() else { return false };
            let mime = d.response().and_then(|r| r.mime_type()).unwrap_or_default();
            let es_principal = d.is_main_frame_main_resource();
            if es_principal && (!d.is_mime_type_supported() || mime.starts_with("video/") || mime == "application/octet-stream") {
                d.download();
                return true;
            }
            false
        });
        // Ventanas nuevas: si las abre un clic del usuario, se cargan en la misma vista; las
        // automáticas (anuncios) se ignoran.
        vista.connect_create(|vista, accion| {
            let accion = accion.clone();
            if accion.is_user_gesture() {
                if let Some(uri) = accion.request().and_then(|r| r.uri()) {
                    vista.load_uri(&uri);
                }
            }
            None
        });

        let atras = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text("Atrás").build();
        let adelante = gtk::Button::builder().icon_name("go-next-symbolic").tooltip_text("Adelante").build();
        let recargar = gtk::Button::builder().icon_name("view-refresh-symbolic").tooltip_text("Recargar").build();
        let direccion = gtk::Entry::builder().hexpand(true).input_purpose(gtk::InputPurpose::Url).build();
        let webs = gtk::DropDown::from_strings(&Fuente::TODAS.iter().map(|f| f.nombre()).collect::<Vec<_>>());
        webs.set_tooltip_text(Some("Ir a una web de fondos"));
        let barra = gtk::Box::builder()
            .spacing(6)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(12)
            .margin_end(12)
            .build();
        let enlazados = gtk::Box::builder().css_classes(["linked"]).build();
        enlazados.append(&atras);
        enlazados.append(&adelante);
        barra.append(&enlazados);
        barra.append(&recargar);
        barra.append(&direccion);
        barra.append(&webs);

        let ayuda = gtk::Label::builder()
            .label("Las descargas se añaden solas a la biblioteca (sin audio). Si una web pide captcha, resuélvelo aquí.")
            .css_classes(["dim-label", "caption"])
            .margin_bottom(6)
            .wrap(true)
            .build();
        let progreso = gtk::ProgressBar::builder().css_classes(["osd"]).visible(false).build();

        let raiz = gtk::Box::new(gtk::Orientation::Vertical, 0);
        raiz.append(&barra);
        raiz.append(&ayuda);
        raiz.append(&progreso);
        raiz.append(&vista);

        {
            let v = vista.clone();
            atras.connect_clicked(move |_| v.go_back());
        }
        {
            let v = vista.clone();
            adelante.connect_clicked(move |_| v.go_forward());
        }
        {
            let v = vista.clone();
            recargar.connect_clicked(move |_| v.reload());
        }
        {
            let v = vista.clone();
            direccion.connect_activate(move |e| {
                let mut url = e.text().trim().to_string();
                if !url.contains("://") {
                    url = format!("https://{url}");
                }
                v.load_uri(&url);
            });
        }
        {
            let v = vista.clone();
            webs.connect_selected_notify(move |d| {
                let f = Fuente::TODAS[d.selected() as usize];
                v.load_uri(&f.url_listado("", 0, None, 1));
            });
        }
        {
            let direccion = direccion.clone();
            vista.connect_uri_notify(move |v| {
                direccion.set_text(&v.uri().unwrap_or_default());
            });
        }
        vista.connect_estimated_load_progress_notify(move |v| {
            let p = v.estimated_load_progress();
            progreso.set_fraction(p);
            progreso.set_visible(p < 1.0);
        });
        // Atrás/adelante disponibles: se revisa en cada carga.
        {
            let actualizar = move |v: &webkit6::WebView| {
                atras.set_sensitive(v.can_go_back());
                adelante.set_sensitive(v.can_go_forward());
            };
            actualizar(&vista);
            vista.connect_load_changed(move |v, _| actualizar(v));
        }

        Rc::new(Self { raiz, vista })
    }

    pub fn abrir(&self, url: &str) {
        self.vista.load_uri(url);
    }

    /// Primera visita: vsthemes, que es la que no tiene integración nativa.
    pub fn cargar_si_vacio(&self) {
        if self.vista.uri().is_none() {
            let _ = glib::idle_add_local_once({
                let v = self.vista.clone();
                move || v.load_uri(&Fuente::VsThemes.url_listado("", 0, None, 1))
            });
        }
    }
}
