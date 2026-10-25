//! Página «Biblioteca»: los fondos de la carpeta de la biblioteca en una cuadrícula, una tarjeta
//! por fondo aunque tenga dos versiones (la original y la de 30 fps, en la subcarpeta «30 fps»).
//! Clic: usar de fondo la versión preferida. Menú de cada fondo: elegir versión, crear o borrar
//! la de 30 fps, convertir, quitar audio, abrir carpeta, borrar. La información de cada vídeo
//! (fps, resolución…) sale de la base de datos (bd.rs) y permite filtrar por fps.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::contexto::{self, Contexto};
use crate::importar::{self, InfoVideo};
use crate::{bd, config, convertir};

/// Un fondo: el vídeo original y, si la hay, su versión de 30 fps. Al menos uno de los dos.
#[derive(Clone, Debug, PartialEq)]
struct Fondo {
    nombre: String,
    original: Option<PathBuf>,
    v30: Option<PathBuf>,
}

impl Fondo {
    /// La que da nombre, miniatura y datos a la tarjeta.
    fn principal(&self) -> &Path {
        self.original.as_deref().or(self.v30.as_deref()).expect("un fondo tiene al menos un vídeo")
    }

    fn rutas(&self) -> Vec<PathBuf> {
        self.original.iter().chain(self.v30.iter()).cloned().collect()
    }

    fn contiene(&self, ruta: &Path) -> bool {
        self.original.as_deref() == Some(ruta) || self.v30.as_deref() == Some(ruta)
    }

    /// La versión que se usa de fondo, según la preferencia.
    fn elegida(&self, preferir_30: bool) -> &Path {
        if preferir_30 {
            self.v30.as_deref().unwrap_or_else(|| self.principal())
        } else {
            self.principal()
        }
    }
}

/// Junta cada original con su versión de 30 fps (mismo nombre). Las versiones sin original
/// (si se borró a mano) son fondos por sí mismas.
fn agrupar(originales: Vec<PathBuf>, versiones: Vec<PathBuf>) -> Vec<Fondo> {
    let mut fondos: Vec<Fondo> = vec![];
    let mut por_nombre: HashMap<String, usize> = HashMap::new();
    for ruta in originales {
        let n = nombre(&ruta);
        por_nombre.entry(n.clone()).or_insert(fondos.len());
        fondos.push(Fondo { nombre: n, original: Some(ruta), v30: None });
    }
    for ruta in versiones {
        let n = nombre(&ruta);
        match por_nombre.get(&n) {
            Some(&i) if fondos[i].v30.is_none() => fondos[i].v30 = Some(ruta),
            _ => fondos.push(Fondo { nombre: n, original: None, v30: Some(ruta) }),
        }
    }
    fondos.sort_by_key(|f| f.nombre.to_lowercase());
    fondos
}

/// Filtro por fps (en el mismo orden que el desplegable).
const FILTROS: &[&str] = &["Todos", "Más de 30 fps", "A 30 fps o menos", "Con las dos versiones", "Sin versión de 30 fps"];

fn pasa_filtro(filtro: u32, fondo: &Fondo, infos: &HashMap<PathBuf, InfoVideo>) -> bool {
    let rapido = fondo.original.as_ref().and_then(|r| infos.get(r)).map(|i| importar::mas_de_30_fps(i.fps));
    match filtro {
        1 => rapido == Some(true),
        2 => fondo.v30.is_some() || rapido == Some(false),
        3 => fondo.original.is_some() && fondo.v30.is_some(),
        4 => fondo.v30.is_none() && rapido == Some(true),
        _ => true,
    }
}

struct Tarjeta {
    fondo: Fondo,
    hijo: gtk::FlowBoxChild,
    marca: gtk::Image,
    detalle: gtk::Label,
    fps: gtk::Label,
    etiqueta: gtk::Overlay,
}

pub struct Biblioteca {
    ctx: Rc<Contexto>,
    pub raiz: gtk::Stack,
    rejilla: gtk::FlowBox,
    filtro: gtk::SearchEntry,
    filtro_fps: gtk::DropDown,
    tarjetas: RefCell<Vec<Tarjeta>>,
    infos: RefCell<HashMap<PathBuf, InfoVideo>>,
    /// Para no aplicar la información de una recarga anterior.
    generacion: Cell<u64>,
    monitores: RefCell<Vec<gio::FileMonitor>>,
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
        let filtro_fps = gtk::DropDown::from_strings(FILTROS);
        filtro_fps.set_tooltip_text(Some("Filtrar por fotogramas por segundo"));
        let barra = gtk::Box::builder()
            .spacing(8)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
            .halign(gtk::Align::Center)
            .build();
        barra.append(&filtro);
        barra.append(&filtro_fps);

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
            filtro_fps,
            tarjetas: RefCell::default(),
            infos: RefCell::default(),
            generacion: Cell::new(0),
            monitores: RefCell::default(),
            recarga: RefCell::default(),
        });

        {
            let debil = Rc::downgrade(&yo);
            yo.rejilla.connect_child_activated(move |_, hijo| {
                let Some(yo) = debil.upgrade() else { return };
                let fondo = yo.tarjetas.borrow().iter().find(|t| &t.hijo == hijo).map(|t| t.fondo.clone());
                if let Some(fondo) = fondo {
                    yo.usar(fondo.elegida(yo.preferir_30()));
                }
            });
        }
        {
            let debil = Rc::downgrade(&yo);
            yo.filtro.connect_search_changed(move |_| {
                if let Some(yo) = debil.upgrade() {
                    yo.aplicar_filtro();
                }
            });
        }
        {
            let debil = Rc::downgrade(&yo);
            yo.filtro_fps.connect_selected_notify(move |_| {
                if let Some(yo) = debil.upgrade() {
                    yo.aplicar_filtro();
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
        yo.recargar();
        yo
    }

    fn carpeta_30(&self) -> PathBuf {
        self.ctx.biblioteca().join(config::CARPETA_30)
    }

    fn preferir_30(&self) -> bool {
        self.ctx.ajustes.string("preferred-version") == "30fps"
    }

    fn usar(&self, ruta: &Path) {
        self.ctx.usar_video(ruta);
        let version = if ruta.starts_with(self.carpeta_30()) { " (30 fps)" } else { "" };
        self.ctx.aviso(&format!("Fondo: {}{version}", nombre(ruta)));
    }

    /// Vigila la biblioteca y su subcarpeta «30 fps» (esta, en cuanto exista).
    fn vigilar_carpetas(self: &Rc<Self>) {
        let dirs = [self.ctx.biblioteca(), self.carpeta_30()];
        let _ = std::fs::create_dir_all(&dirs[0]);
        let mut monitores = self.monitores.borrow_mut();
        for dir in dirs.iter().skip(monitores.len()) {
            if !dir.is_dir() {
                break;
            }
            let Ok(monitor) =
                gio::File::for_path(dir).monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
            else {
                break;
            };
            let debil = Rc::downgrade(self);
            monitor.connect_changed(move |_, _, _, _| {
                if let Some(yo) = debil.upgrade() {
                    yo.recargar_pronto();
                }
            });
            monitores.push(monitor);
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

    fn videos_en(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && config::es_video(p) && !config::es_temporal(p))
            .collect();
        v.sort_by_key(|p| nombre(p).to_lowercase());
        v
    }

    fn fondos(&self) -> Vec<Fondo> {
        agrupar(Self::videos_en(&self.ctx.biblioteca()), Self::videos_en(&self.carpeta_30()))
    }

    /// Todos los vídeos, originales y versiones.
    fn videos(&self) -> Vec<PathBuf> {
        self.fondos().iter().flat_map(Fondo::rutas).collect()
    }

    pub fn recargar(self: &Rc<Self>) {
        self.vigilar_carpetas();
        while let Some(hijo) = self.rejilla.first_child() {
            self.rejilla.remove(&hijo);
        }
        self.tarjetas.borrow_mut().clear();
        let fondos = self.fondos();
        self.raiz.set_visible_child_name(if fondos.is_empty() { "vacia" } else { "lista" });
        let rutas: Vec<PathBuf> = fondos.iter().flat_map(Fondo::rutas).collect();
        for fondo in fondos {
            let tarjeta = self.tarjeta(fondo);
            self.rejilla.append(&tarjeta.hijo);
            self.tarjetas.borrow_mut().push(tarjeta);
        }
        self.marcar_actual();
        self.actualizar_datos();

        // Datos de los vídeos: casi siempre de la base de datos; ffprobe solo para los nuevos.
        let generacion = self.generacion.get() + 1;
        self.generacion.set(generacion);
        let yo = self.clone();
        glib::spawn_future_local(async move {
            let infos = gio::spawn_blocking(move || bd::sincronizar(&rutas)).await.unwrap_or_default();
            if yo.generacion.get() == generacion {
                yo.infos.replace(infos);
                yo.actualizar_datos();
            }
        });
    }

    /// Detalle y etiqueta de fps de cada tarjeta, y el filtro (que depende de los fps).
    fn actualizar_datos(&self) {
        let infos = self.infos.borrow();
        for t in self.tarjetas.borrow().iter() {
            if let Some(info) = infos.get(t.fondo.principal()) {
                t.detalle.set_label(&info.resumen());
                t.detalle.set_tooltip_text(Some(&info.resumen()));
            }
            let fps = |r: &Option<PathBuf>| r.as_ref().and_then(|r| infos.get(r)).map(|i| format!("{:.0}", i.fps));
            let texto = match (fps(&t.fondo.original), fps(&t.fondo.v30)) {
                (Some(o), Some(v)) => Some(format!("{o} · {v} fps")),
                (Some(f), None) | (None, Some(f)) => Some(format!("{f} fps")),
                (None, None) => None,
            };
            t.etiqueta.set_visible(texto.is_some());
            t.fps.set_label(texto.as_deref().unwrap_or_default());
            t.fps.set_tooltip_text(match (&t.fondo.original, &t.fondo.v30) {
                (Some(_), Some(_)) => Some("Original y versión de 30 fps"),
                (None, Some(_)) => Some("Solo la versión de 30 fps"),
                _ => None,
            });
        }
        drop(infos);
        self.aplicar_filtro();
    }

    fn aplicar_filtro(&self) {
        let texto = self.filtro.text().to_lowercase();
        let filtro = self.filtro_fps.selected();
        let infos = self.infos.borrow();
        for t in self.tarjetas.borrow().iter() {
            let por_nombre = texto.is_empty() || t.fondo.nombre.to_lowercase().contains(&texto);
            t.hijo.set_visible(por_nombre && pasa_filtro(filtro, &t.fondo, &infos));
        }
    }

    fn marcar_actual(&self) {
        let actual = self.ctx.video_actual();
        for t in self.tarjetas.borrow().iter() {
            let es = t.fondo.contiene(&actual);
            t.marca.set_visible(es);
            t.marca.set_tooltip_text(match (es, t.fondo.v30.as_deref() == Some(actual.as_path())) {
                (true, true) => Some("De fondo: la versión de 30 fps"),
                (true, false) if t.fondo.v30.is_some() => Some("De fondo: la versión original"),
                _ => None,
            });
            if es {
                t.hijo.add_css_class("fondo-actual");
            } else {
                t.hijo.remove_css_class("fondo-actual");
            }
        }
    }

    fn tarjeta(self: &Rc<Self>, fondo: Fondo) -> Tarjeta {
        let imagen = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .width_request(240)
            .height_request(135)
            .css_classes(["card", "miniatura"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        contexto::cargar_miniatura_local(&imagen, fondo.principal());
        let marca = gtk::Image::builder()
            .icon_name("object-select-symbolic")
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(6)
            .margin_end(6)
            .css_classes(["marca-actual"])
            .visible(false)
            .build();
        let (etiqueta, fps) = etiqueta_fps();
        let encima = gtk::Overlay::builder().child(&imagen).build();
        encima.add_overlay(&marca);
        encima.add_overlay(&etiqueta);

        let titulo = gtk::Label::builder()
            .label(&fondo.nombre)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(24)
            .build();
        let detalle = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label", "caption"])
            .build();
        let textos = gtk::Box::builder().orientation(gtk::Orientation::Vertical).hexpand(true).build();
        textos.append(&titulo);
        textos.append(&detalle);

        let boton = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu(&fondo))
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
        hijo.insert_action_group("tarjeta", Some(&self.acciones(&fondo)));
        Tarjeta { fondo, hijo, marca, detalle, fps, etiqueta }
    }

    fn acciones(self: &Rc<Self>, fondo: &Fondo) -> gio::SimpleActionGroup {
        let grupo = gio::SimpleActionGroup::new();
        let accion = |nombre: &str, f: Box<dyn Fn(&Rc<Biblioteca>, &Fondo)>| {
            let a = gio::SimpleAction::new(nombre, None);
            let debil = Rc::downgrade(self);
            let fondo = fondo.clone();
            a.connect_activate(move |_, _| {
                if let Some(yo) = debil.upgrade() {
                    f(&yo, &fondo);
                }
            });
            grupo.add_action(&a);
        };
        accion("usar", Box::new(|yo, f| yo.usar(f.elegida(yo.preferir_30()))));
        accion("usar-original", Box::new(|yo, f| yo.usar(f.elegida(false))));
        accion("usar-30", Box::new(|yo, f| yo.usar(f.elegida(true))));
        accion("crear-30", Box::new(|yo, f| yo.crear_version_30(f)));
        accion("rehacer-30", Box::new(|yo, f| yo.crear_versiones_30(f.original.iter().cloned().collect())));
        accion("borrar-30", Box::new(|yo, f| yo.borrar_version_30(f)));
        accion("convertir", Box::new(|yo, f| convertir::abrir(yo.ctx.clone(), f.principal())));
        accion("sin-audio", Box::new(|yo, f| yo.quitar_audio(f.rutas())));
        accion(
            "carpeta",
            Box::new(|yo, f| {
                let lanzador = gtk::FileLauncher::new(Some(&gio::File::for_path(f.principal())));
                lanzador.open_containing_folder(Some(&yo.ctx.ventana), gio::Cancellable::NONE, |_| {});
            }),
        );
        accion("borrar", Box::new(|yo, f| yo.borrar(f)));
        grupo
    }

    /// Pide confirmación y mueve `rutas` a la papelera. Devuelve si se borraron.
    async fn a_la_papelera(&self, titulo: &str, cuerpo: &str, rutas: &[PathBuf]) -> bool {
        let dialogo = adw::AlertDialog::new(Some(titulo), Some(cuerpo));
        dialogo.add_responses(&[("cancelar", "Cancelar"), ("borrar", "Borrar")]);
        dialogo.set_response_appearance("borrar", adw::ResponseAppearance::Destructive);
        dialogo.set_default_response(Some("cancelar"));
        if dialogo.choose_future(Some(&self.ctx.ventana)).await != "borrar" {
            return false;
        }
        for ruta in rutas {
            if let Err(e) = gio::File::for_path(ruta).trash(gio::Cancellable::NONE) {
                self.ctx.aviso(&format!("No se pudo borrar: {e}"));
                return false;
            }
        }
        true
    }

    fn borrar(self: &Rc<Self>, fondo: &Fondo) {
        let yo = self.clone();
        let fondo = fondo.clone();
        glib::spawn_future_local(async move {
            let cuerpo = if fondo.original.is_some() && fondo.v30.is_some() {
                "El vídeo y su versión de 30 fps se mueven a la papelera."
            } else {
                "El vídeo se mueve a la papelera."
            };
            if !yo.a_la_papelera(&format!("¿Borrar «{}»?", fondo.nombre), cuerpo, &fondo.rutas()).await {
                return;
            }
            if fondo.contiene(&yo.ctx.video_actual()) {
                let _ = yo.ctx.ajustes.set_boolean("enabled", false);
            }
            yo.recargar_pronto();
        });
    }

    fn borrar_version_30(self: &Rc<Self>, fondo: &Fondo) {
        let (Some(original), Some(v30)) = (fondo.original.clone(), fondo.v30.clone()) else { return };
        let yo = self.clone();
        let nombre = fondo.nombre.clone();
        glib::spawn_future_local(async move {
            let titulo = format!("¿Borrar la versión de 30 fps de «{nombre}»?");
            if !yo.a_la_papelera(&titulo, "Se mueve a la papelera; el original se queda.", &[v30.clone()]).await {
                return;
            }
            if yo.ctx.video_actual() == v30 {
                yo.ctx.usar_video(&original);
            }
            yo.recargar_pronto();
        });
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

    fn crear_version_30(self: &Rc<Self>, fondo: &Fondo) {
        let Some(original) = fondo.original.clone() else { return };
        if let Some(info) = self.infos.borrow().get(&original).filter(|i| !importar::mas_de_30_fps(i.fps)) {
            self.ctx.aviso(&format!("«{}» ya va a {:.0} fps", fondo.nombre, info.fps));
            return;
        }
        self.crear_versiones_30(vec![original]);
    }

    /// Crea la versión de 30 fps de los originales de la biblioteca que vayan más rápido y aún
    /// no la tengan (pide confirmación). Los originales se conservan.
    pub fn crear_versiones_30_todas(self: &Rc<Self>) {
        let yo = self.clone();
        let originales: Vec<PathBuf> = self.fondos().into_iter().filter(|f| f.v30.is_none()).filter_map(|f| f.original).collect();
        glib::spawn_future_local(async move {
            let rapidos: Vec<(PathBuf, f64)> = gio::spawn_blocking(move || {
                let infos = bd::sincronizar(&originales);
                originales
                    .into_iter()
                    .filter_map(|r| {
                        let fps = infos.get(&r)?.fps;
                        importar::mas_de_30_fps(fps).then_some((r, fps))
                    })
                    .collect()
            })
            .await
            .unwrap_or_default();
            if rapidos.is_empty() {
                yo.ctx.aviso("No hay vídeos de más de 30 fps sin su versión de 30");
                return;
            }
            let dialogo = adw::AlertDialog::new(
                Some(&format!("¿Crear la versión de 30 fps de {} vídeo(s)?", rapidos.len())),
                Some(&format!(
                    "Los originales se conservan: las versiones van a la subcarpeta «{}» de la \
                     biblioteca y ocupan más disco. Puede tardar bastante, sobre todo si la \
                     conversión va por la CPU.\n\n{}",
                    config::CARPETA_30,
                    rapidos
                        .iter()
                        .map(|(r, f)| format!("{} ({f:.0} fps)", nombre(r)))
                        .collect::<Vec<_>>()
                        .join("\n")
                )),
            );
            dialogo.add_responses(&[("cancelar", "Cancelar"), ("crear", "Crear")]);
            dialogo.set_response_appearance("crear", adw::ResponseAppearance::Suggested);
            dialogo.set_default_response(Some("cancelar"));
            if dialogo.choose_future(Some(&yo.ctx.ventana)).await == "crear" {
                yo.crear_versiones_30(rapidos.into_iter().map(|(r, _)| r).collect());
            }
        });
    }

    /// Uno detrás de otro, con un aviso que enseña el avance y permite cancelar entre vídeo y
    /// vídeo.
    fn crear_versiones_30(self: &Rc<Self>, originales: Vec<PathBuf>) {
        let yo = self.clone();
        glib::spawn_future_local(async move {
            let cancelado = Rc::new(Cell::new(false));
            let aviso = adw::Toast::builder().timeout(0).button_label("Cancelar").build();
            {
                let cancelado = cancelado.clone();
                aviso.connect_button_clicked(move |a| {
                    cancelado.set(true);
                    a.set_title("Cancelando al acabar el vídeo actual…");
                });
            }
            yo.ctx.avisos.add_toast(aviso.clone());
            let total = originales.len();
            let (mut hechos, mut errores) = (0, 0);
            for (n, original) in originales.into_iter().enumerate() {
                if cancelado.get() {
                    break;
                }
                aviso.set_title(&format!("Creando la versión de 30 fps {}/{total}: {}", n + 1, nombre(&original)));
                let o = original.clone();
                let calidad = yo.ctx.calidad_30();
                let r = gio::spawn_blocking(move || importar::crear_version_30(&o, calidad))
                    .await
                    .unwrap_or_else(|_| Err("fallo interno".into()));
                match r {
                    Ok(v30) => {
                        hechos += 1;
                        // Si es el fondo actual y se prefiere la de 30, pasar a ella ya.
                        if yo.preferir_30() && yo.ctx.video_actual() == original {
                            yo.ctx.usar_video(&v30);
                        }
                    }
                    Err(e) => {
                        crate::depurar!("versión de 30 fps de {}: {e}", original.display());
                        errores += 1;
                    }
                }
                yo.recargar();
            }
            aviso.dismiss();
            yo.ctx.aviso(&match (hechos, errores, cancelado.get()) {
                (1, 0, _) if total == 1 => "Versión de 30 fps creada".to_string(),
                (h, 0, false) => format!("{h} versión(es) de 30 fps creadas"),
                (h, 0, true) => format!("Cancelado: {h} de {total} versión(es) de 30 fps creadas"),
                (h, e, _) => format!("{h} versión(es) de 30 fps creadas; {e} con errores"),
            });
        });
    }
}

/// Etiqueta roja con forma de etiqueta (punta a la izquierda y un agujero por el que se ve la
/// miniatura) para la esquina inferior derecha de la miniatura. El texto (el Label) manda en
/// el tamaño; la forma se dibuja debajo, a su medida.
fn etiqueta_fps() -> (gtk::Overlay, gtk::Label) {
    let forma = gtk::DrawingArea::new();
    forma.set_draw_func(|_, cr, ancho, alto| {
        let (w, h) = (ancho as f64, alto as f64);
        let punta = h * 0.45;
        let r = 3.0;
        cr.move_to(punta, 0.0);
        cr.arc(w - r, r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(w - r, h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.line_to(punta, h);
        cr.line_to(0.0, h / 2.0);
        cr.close_path();
        cr.set_source_rgb(0.878, 0.106, 0.141); // rojo de Adwaita (#e01b24)
        let _ = cr.fill();
        // El agujero, transparente de verdad.
        cr.set_operator(gtk::cairo::Operator::Clear);
        cr.arc(punta * 0.9, h / 2.0, 2.2, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    });
    let texto = gtk::Label::builder().css_classes(["etiqueta-fps", "caption"]).build();
    let etiqueta = gtk::Overlay::builder()
        .child(&forma)
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(6)
        .margin_bottom(6)
        .visible(false)
        .build();
    etiqueta.add_overlay(&texto);
    etiqueta.set_measure_overlay(&texto, true);
    (etiqueta, texto)
}

/// Menú ⋮ de un fondo; las opciones de versiones dependen de las que tenga.
fn menu(fondo: &Fondo) -> gio::Menu {
    let menu = gio::Menu::new();
    let usar = gio::Menu::new();
    usar.append(Some("Usar de fondo"), Some("tarjeta.usar"));
    if fondo.original.is_some() && fondo.v30.is_some() {
        usar.append(Some("Usar la versión original"), Some("tarjeta.usar-original"));
        usar.append(Some("Usar la versión de 30 fps"), Some("tarjeta.usar-30"));
    }
    menu.append_section(None, &usar);
    let versiones = gio::Menu::new();
    match (&fondo.original, &fondo.v30) {
        (Some(_), None) => versiones.append(Some("Crear versión de 30 fps"), Some("tarjeta.crear-30")),
        (Some(_), Some(_)) => {
            // Rehacer: con la calidad de Ajustes (crear_version_30 la sustituye al terminar).
            versiones.append(Some("Rehacer la versión de 30 fps"), Some("tarjeta.rehacer-30"));
            versiones.append(Some("Borrar la versión de 30 fps"), Some("tarjeta.borrar-30"));
        }
        _ => {}
    }
    if versiones.n_items() > 0 {
        menu.append_section(None, &versiones);
    }
    let resto = gio::Menu::new();
    resto.append(Some("Convertir / recortar…"), Some("tarjeta.convertir"));
    resto.append(Some("Quitar audio"), Some("tarjeta.sin-audio"));
    resto.append(Some("Abrir carpeta"), Some("tarjeta.carpeta"));
    resto.append(Some("Borrar"), Some("tarjeta.borrar"));
    menu.append_section(None, &resto);
    menu
}

fn nombre(ruta: &Path) -> String {
    ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn rutas(v: &[&str]) -> Vec<PathBuf> {
        v.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn agrupa_originales_y_versiones() {
        let fondos = agrupar(
            rutas(&["/b/Ciudad.webm", "/b/frieren.mp4"]),
            rutas(&["/b/30 fps/frieren.mp4", "/b/30 fps/huerfano.mp4"]),
        );
        assert_eq!(fondos.len(), 3);
        assert_eq!(fondos[0].nombre, "Ciudad");
        assert_eq!(fondos[0].v30, None);
        assert_eq!(fondos[1].nombre, "frieren");
        assert_eq!(fondos[1].v30.as_deref(), Some(Path::new("/b/30 fps/frieren.mp4")));
        assert_eq!(fondos[1].elegida(true), Path::new("/b/30 fps/frieren.mp4"));
        assert_eq!(fondos[1].elegida(false), Path::new("/b/frieren.mp4"));
        assert_eq!(fondos[2].original, None);
        assert_eq!(fondos[2].elegida(false), Path::new("/b/30 fps/huerfano.mp4"));
        assert_eq!(fondos[0].elegida(true), Path::new("/b/Ciudad.webm"));
    }

    #[test]
    fn filtra_por_fps() {
        let fondos = agrupar(rutas(&["/b/a.mp4", "/b/b.mp4", "/b/c.mp4"]), rutas(&["/b/30 fps/a.mp4"]));
        let info = |fps| InfoVideo { fps, ..Default::default() };
        let infos: HashMap<PathBuf, InfoVideo> = [
            (PathBuf::from("/b/a.mp4"), info(60.0)),
            (PathBuf::from("/b/30 fps/a.mp4"), info(30.0)),
            (PathBuf::from("/b/b.mp4"), info(59.94)),
            (PathBuf::from("/b/c.mp4"), info(29.97)),
        ]
        .into();
        let que_pasan = |filtro| {
            fondos.iter().filter(|f| pasa_filtro(filtro, f, &infos)).map(|f| f.nombre.as_str()).collect::<Vec<_>>()
        };
        assert_eq!(que_pasan(0), ["a", "b", "c"]);
        assert_eq!(que_pasan(1), ["a", "b"]);
        assert_eq!(que_pasan(2), ["a", "c"]);
        assert_eq!(que_pasan(3), ["a"]);
        assert_eq!(que_pasan(4), ["b"]);
    }
}
