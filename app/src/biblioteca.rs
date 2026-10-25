//! Página «Biblioteca»: los vídeos de la carpeta de la biblioteca en una cuadrícula. Clic: usar
//! de fondo. Menú de cada vídeo: convertir, quitar audio, abrir carpeta, borrar.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::contexto::{self, Contexto};
use crate::{config, convertir, importar};

pub struct Biblioteca {
    ctx: Rc<Contexto>,
    pub raiz: gtk::Stack,
    rejilla: gtk::FlowBox,
    filtro: gtk::SearchEntry,
    tarjetas: RefCell<Vec<(PathBuf, gtk::FlowBoxChild, gtk::Image)>>,
    _monitor: RefCell<Option<gio::FileMonitor>>,
    recarga: RefCell<Option<glib::SourceId>>,
}

impl Biblioteca {
    pub fn new(ctx: Rc<Contexto>, descargas: Rc<crate::descargas::Descargas>) -> Rc<Self> {
        let rejilla = gtk::FlowBox::builder()
            .valign(gtk::Align::Start)
            .homogeneous(true)
            .min_children_per_line(3)
            .max_children_per_line(8)
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .column_spacing(12)
            .row_spacing(12)
            .margin_top(12)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        let filtro = gtk::SearchEntry::builder()
            .placeholder_text("Filtrar la biblioteca")
            .hexpand(true)
            .max_width_chars(40)
            .build();
        let barra = gtk::Box::builder()
            .spacing(8)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .halign(gtk::Align::Center)
            .build();
        barra.append(&filtro);

        let caja = gtk::Box::new(gtk::Orientation::Vertical, 0);
        caja.append(&barra);
        caja.append(&gtk::ScrolledWindow::builder().vexpand(true).hscrollbar_policy(gtk::PolicyType::Never).child(&rejilla).build());

        let vacia = adw::StatusPage::builder()
            .icon_name("video-x-generic-symbolic")
            .title("La biblioteca está vacía")
            .description("Busca fondos en «Explorar», arrastra aquí vídeos o paquetes de Wallpaper Engine (.zip), o añádelos con el botón +.")
            .build();
        let explorar = gtk::Button::builder()
            .label("Explorar fondos")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .action_name("win.ir-explorar")
            .build();
        vacia.set_child(Some(&explorar));

        let raiz = gtk::Stack::new();
        raiz.add_named(&caja, Some("lista"));
        raiz.add_named(&vacia, Some("vacia"));

        let yo = Rc::new(Self {
            ctx: ctx.clone(),
            raiz,
            rejilla,
            filtro,
            tarjetas: RefCell::default(),
            _monitor: RefCell::default(),
            recarga: RefCell::default(),
        });

        {
            let debil = Rc::downgrade(&yo);
            yo.rejilla.connect_child_activated(move |_, hijo| {
                let Some(yo) = debil.upgrade() else { return };
                let ruta = yo.tarjetas.borrow().iter().find(|t| &t.1 == hijo).map(|t| t.0.clone());
                if let Some(ruta) = ruta {
                    yo.ctx.usar_video(&ruta);
                    yo.ctx.aviso(&format!("Fondo: {}", nombre(&ruta)));
                }
            });
        }
        {
            let debil = Rc::downgrade(&yo);
            yo.filtro.connect_search_changed(move |f| {
                let Some(yo) = debil.upgrade() else { return };
                let texto = f.text().to_lowercase();
                for (ruta, hijo, _) in yo.tarjetas.borrow().iter() {
                    hijo.set_visible(texto.is_empty() || nombre(ruta).to_lowercase().contains(&texto));
                }
            });
        }
        {
            let debil = Rc::downgrade(&yo);
            ctx.ajustes.connect_changed(Some("video"), move |_, _| {
                if let Some(yo) = debil.upgrade() {
                    yo.marcar_actual();
                }
            });
        }
        {
            let debil = Rc::downgrade(&yo);
            ctx.conectar_biblioteca(move || {
                if let Some(yo) = debil.upgrade() {
                    yo.recargar_pronto();
                }
            });
        }
        {
            // Soltar ficheros en la ventana: importarlos.
            let destino = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), gtk::gdk::DragAction::COPY);
            destino.connect_drop(move |_, valor, _, _| {
                let Ok(lista) = valor.get::<gtk::gdk::FileList>() else { return false };
                let rutas: Vec<PathBuf> = lista.files().iter().filter_map(|f| f.path()).collect();
                descargas.importar_locales(rutas);
                true
            });
            ctx.ventana.add_controller(destino);
        }
        yo.vigilar_carpeta();
        yo.recargar();
        yo
    }

    fn vigilar_carpeta(self: &Rc<Self>) {
        let dir = self.ctx.biblioteca();
        let _ = std::fs::create_dir_all(&dir);
        let monitor = gio::File::for_path(&dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE);
        if let Ok(monitor) = monitor {
            let debil = Rc::downgrade(self);
            monitor.connect_changed(move |_, _, _, _| {
                if let Some(yo) = debil.upgrade() {
                    yo.recargar_pronto();
                }
            });
            self._monitor.replace(Some(monitor));
        }
    }

    /// Recarga agrupando cambios seguidos (copias grandes generan muchos eventos).
    fn recargar_pronto(self: &Rc<Self>) {
        if let Some(id) = self.recarga.take() {
            id.remove();
        }
        let debil = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if let Some(yo) = debil.upgrade() {
                yo.recarga.replace(None);
                yo.recargar();
            }
        });
        self.recarga.replace(Some(id));
    }

    fn videos(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(self.ctx.biblioteca())
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && config::es_video(p) && !p.to_string_lossy().contains(".tmp."))
            .collect();
        v.sort_by_key(|p| nombre(p).to_lowercase());
        v
    }

    pub fn recargar(self: &Rc<Self>) {
        while let Some(hijo) = self.rejilla.first_child() {
            self.rejilla.remove(&hijo);
        }
        self.tarjetas.borrow_mut().clear();
        let videos = self.videos();
        self.raiz.set_visible_child_name(if videos.is_empty() { "vacia" } else { "lista" });
        for ruta in videos {
            let (hijo, marca) = self.tarjeta(&ruta);
            self.rejilla.append(&hijo);
            self.tarjetas.borrow_mut().push((ruta, hijo, marca));
        }
        self.marcar_actual();
        self.filtro.emit_by_name::<()>("search-changed", &[]);
    }

    fn marcar_actual(&self) {
        let actual = self.ctx.video_actual();
        for (ruta, hijo, marca) in self.tarjetas.borrow().iter() {
            let es = *ruta == actual;
            marca.set_visible(es);
            if es {
                hijo.add_css_class("fondo-actual");
            } else {
                hijo.remove_css_class("fondo-actual");
            }
        }
    }

    fn tarjeta(self: &Rc<Self>, ruta: &Path) -> (gtk::FlowBoxChild, gtk::Image) {
        let imagen = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(240)
            .height_request(135)
            .css_classes(["card", "miniatura"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        contexto::cargar_miniatura_local(&imagen, ruta);
        let marca = gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .margin_end(6)
            .css_classes(["marca-actual"])
            .visible(false)
            .build();
        let encima = gtk::Overlay::builder().child(&imagen).build();
        encima.add_overlay(&marca);

        let titulo = gtk::Label::builder()
            .label(nombre(ruta))
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(24)
            .build();
        let detalle = gtk::Label::builder().xalign(0.0).css_classes(["dim-label", "caption"]).build();
        {
            let ruta = ruta.to_path_buf();
            let detalle = detalle.downgrade();
            glib::spawn_future_local(async move {
                let info = gio::spawn_blocking(move || importar::info(&ruta)).await;
                if let (Ok(Ok(info)), Some(d)) = (info, detalle.upgrade()) {
                    d.set_label(&info.resumen());
                    d.set_tooltip_text(Some(&info.resumen()));
                }
            });
        }
        let textos = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        textos.append(&titulo);
        textos.append(&detalle);

        let menu = gio::Menu::new();
        menu.append(Some("Usar de fondo"), Some("tarjeta.usar"));
        menu.append(Some("Convertir / recortar…"), Some("tarjeta.convertir"));
        menu.append(Some("Quitar audio"), Some("tarjeta.sin-audio"));
        menu.append(Some("Abrir carpeta"), Some("tarjeta.carpeta"));
        menu.append(Some("Borrar"), Some("tarjeta.borrar"));
        let boton = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu)
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        let pie = gtk::Box::builder().spacing(4).build();
        pie.append(&textos);
        pie.append(&boton);

        let caja = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
        caja.append(&encima);
        caja.append(&pie);
        let hijo = gtk::FlowBoxChild::builder().child(&caja).build();
        hijo.insert_action_group("tarjeta", Some(&self.acciones(ruta)));
        (hijo, marca)
    }

    fn acciones(self: &Rc<Self>, ruta: &Path) -> gio::SimpleActionGroup {
        let grupo = gio::SimpleActionGroup::new();
        let accion = |nombre: &str, f: Box<dyn Fn(&Rc<Biblioteca>, &Path)>| {
            let a = gio::SimpleAction::new(nombre, None);
            let debil = Rc::downgrade(self);
            let ruta = ruta.to_path_buf();
            a.connect_activate(move |_, _| {
                if let Some(yo) = debil.upgrade() {
                    f(&yo, &ruta);
                }
            });
            grupo.add_action(&a);
        };
        accion("usar", Box::new(|yo, ruta| yo.ctx.usar_video(ruta)));
        accion("convertir", Box::new(|yo, ruta| convertir::abrir(yo.ctx.clone(), ruta)));
        accion("sin-audio", Box::new(|yo, ruta| yo.quitar_audio(vec![ruta.to_path_buf()])));
        accion(
            "carpeta",
            Box::new(|yo, ruta| {
                let lanzador = gtk::FileLauncher::new(Some(&gio::File::for_path(ruta)));
                lanzador.open_containing_folder(Some(&yo.ctx.ventana), gio::Cancellable::NONE, |_| {});
            }),
        );
        accion("borrar", Box::new(|yo, ruta| yo.borrar(ruta)));
        grupo
    }

    fn borrar(self: &Rc<Self>, ruta: &Path) {
        let dialogo = adw::AlertDialog::new(
            Some(&format!("¿Borrar «{}»?", nombre(ruta))),
            Some("El vídeo se mueve a la papelera."),
        );
        dialogo.add_responses(&[("cancelar", "Cancelar"), ("borrar", "Borrar")]);
        dialogo.set_response_appearance("borrar", adw::ResponseAppearance::Destructive);
        dialogo.set_default_response(Some("cancelar"));
        let yo = self.clone();
        let ruta = ruta.to_path_buf();
        dialogo.connect_response(None, move |_, respuesta| {
            if respuesta != "borrar" {
                return;
            }
            match gio::File::for_path(&ruta).trash(gio::Cancellable::NONE) {
                Ok(()) => {
                    if yo.ctx.video_actual() == ruta {
                        let _ = yo.ctx.ajustes.set_boolean("enabled", false);
                    }
                    yo.recargar_pronto();
                }
                Err(e) => yo.ctx.aviso(&format!("No se pudo borrar: {e}")),
            }
        });
        dialogo.present(Some(&self.ctx.ventana));
    }

    pub fn quitar_audio(self: &Rc<Self>, rutas: Vec<PathBuf>) {
        let yo = self.clone();
        glib::spawn_future_local(async move {
            yo.ctx.aviso("Quitando el audio…");
            let (hechos, errores) = gio::spawn_blocking(move || {
                let mut hechos = 0;
                let mut errores = 0;
                for r in rutas {
                    match importar::info(&r) {
                        Ok(i) if i.audio => match importar::quitar_audio(&r) {
                            Ok(()) => hechos += 1,
                            Err(_) => errores += 1,
                        },
                        _ => {}
                    }
                }
                (hechos, errores)
            })
            .await
            .unwrap_or((0, 1));
            yo.ctx.aviso(&match (hechos, errores) {
                (0, 0) => "Ningún vídeo tenía audio".to_string(),
                (h, 0) => format!("Audio quitado de {h} vídeo(s)"),
                (h, e) => format!("Audio quitado de {h} vídeo(s); {e} con errores"),
            });
            yo.recargar();
        });
    }

    pub fn quitar_audio_todos(self: &Rc<Self>) {
        self.quitar_audio(self.videos());
    }
}

fn nombre(ruta: &Path) -> String {
    ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}
