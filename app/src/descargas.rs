//! Cola de descargas en la ventana: un botón en la barra de título con la lista. El trabajo lo
//! hace el gestor de descargas (gestor.rs), un proceso aparte; aquí se encola, se le piden cosas
//! (pausar, reanudar, subir, bajar, cancelar, reintentar, quitar) y se enseña cómo va leyendo la
//! cola (cola.rs) cada medio segundo. Las descargas del navegador integrado las hace WebKit
//! dentro de la app; al acabar, su fichero entra en la cola para importarlo.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use rusqlite::Connection;

use crate::cola::{self, Encolado, Trabajo};
use crate::contexto::Contexto;
use crate::{config, gestor};

/// Segundos que se queda en la lista una descarga terminada bien.
const QUEDARSE_TRAS_TERMINAR: i64 = 20;

pub struct Descargas {
    ctx: Rc<Contexto>,
    pub boton: gtk::MenuButton,
    lista: gtk::ListBox,
    lista_navegador: gtk::ListBox,
    limpiar: gtk::Button,
    con: RefCell<Option<Connection>>,
    filas: RefCell<HashMap<i64, Fila>>,
    /// Descargas del navegador en curso (no están en la cola hasta que acaban).
    del_navegador: Cell<u32>,
}

/// Los controles de una fila de la lista.
struct Fila {
    fila: gtk::ListBoxRow,
    nombre: gtk::Label,
    barra: gtk::ProgressBar,
    texto: gtk::Label,
    subir: gtk::Button,
    bajar: gtk::Button,
    pausar: gtk::Button,
    reanudar: gtk::Button,
    reintentar: gtk::Button,
    cancelar: gtk::Button,
    quitar: gtk::Button,
    ultimo: RefCell<Option<Trabajo>>,
}

#[derive(Clone, Copy)]
enum Accion {
    Subir,
    Bajar,
    Pausar,
    Reanudar,
    Reintentar,
    Cancelar,
    Quitar,
}

fn boton(icono: &str, ayuda: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icono)
        .tooltip_text(ayuda)
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build()
}

/// Nombre, barra y texto de estado; los botones van a la derecha.
fn caja_fila(nombre: &gtk::Label, barra: &gtk::ProgressBar, texto: &gtk::Label, botones: &[&gtk::Button]) -> gtk::Box {
    let textos = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).build();
    textos.append(nombre);
    textos.append(barra);
    textos.append(texto);
    let caja = gtk::Box::builder()
        .spacing(2)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(10)
        .margin_end(6)
        .build();
    caja.append(&textos);
    for b in botones {
        caja.append(*b);
    }
    caja
}

fn etiqueta_nombre(texto: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(texto)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(36)
        .tooltip_text(texto)
        .build()
}

fn etiqueta_estado() -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .max_width_chars(44)
        .css_classes(["dim-label", "caption"])
        .build()
}

fn tamano(hecho: u64, total: Option<u64>) -> String {
    match total {
        Some(t) if t > 0 => format!("{} de {}", config::tamano_legible(hecho), config::tamano_legible(t)),
        _ => config::tamano_legible(hecho),
    }
}

/// Texto de estado de un trabajo.
fn describir(t: &Trabajo) -> String {
    let descarga = t.url.is_some();
    match t.estado.as_str() {
        cola::EN_COLA if t.hecho > 0 => format!("En cola · {}", tamano(t.hecho, t.total)),
        cola::EN_COLA => "En cola".into(),
        cola::DESCARGANDO => match t.peticion.as_deref() {
            Some(cola::PEDIR_PAUSA) => "Pausando…".into(),
            Some(_) => "Cancelando…".into(),
            None if t.hecho == 0 => "Empezando…".into(),
            None => tamano(t.hecho, t.total),
        },
        cola::PAUSADA => format!("En pausa · {}", tamano(t.hecho, t.total)),
        cola::POR_IMPORTAR if descarga => "Descargada · esperando para importar".into(),
        cola::POR_IMPORTAR => "Esperando para importar".into(),
        cola::IMPORTANDO if t.ajustes.optimizar.is_some() => "Adaptando el vídeo a la pantalla (puede tardar)…".into(),
        cola::IMPORTANDO if t.mover => "Preparando el vídeo (quitando audio, comprobando códec)…".into(),
        cola::IMPORTANDO => "Importando…".into(),
        cola::LISTA => format!("Listo · {}", t.mensaje.as_deref().unwrap_or("")),
        cola::ERROR => format!("Error: {}", t.mensaje.as_deref().unwrap_or("desconocido")),
        cola::CANCELADA => "Cancelada".into(),
        otro => otro.into(),
    }
}

impl Descargas {
    pub fn new(ctx: Rc<Contexto>) -> Rc<Self> {
        let lista = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let lista_navegador = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .visible(false)
            .build();
        let titulo = gtk::Label::builder().label("Cola de descargas").xalign(0.0).hexpand(true).css_classes(["heading"]).build();
        let limpiar = gtk::Button::builder()
            .label("Quitar terminadas")
            .css_classes(["flat"])
            .sensitive(false)
            .build();
        let cabecera = gtk::Box::builder().spacing(6).margin_start(4).build();
        cabecera.append(&titulo);
        cabecera.append(&limpiar);
        let contenido = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).build();
        contenido.append(&lista);
        contenido.append(&lista_navegador);
        let marco = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(480)
            .child(&contenido)
            .build();
        let caja = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).width_request(460).build();
        caja.append(&cabecera);
        caja.append(&marco);
        let popover = gtk::Popover::builder().child(&caja).build();
        let boton = gtk::MenuButton::builder()
            .icon_name("folder-download-symbolic")
            .tooltip_text("Descargas")
            .popover(&popover)
            .visible(false)
            .build();
        let yo = Rc::new(Self {
            ctx,
            boton,
            lista,
            lista_navegador,
            limpiar,
            con: RefCell::default(),
            filas: RefCell::default(),
            del_navegador: Cell::new(0),
        });
        {
            let debil = Rc::downgrade(&yo);
            yo.limpiar.connect_clicked(move |_| {
                if let Some(yo) = debil.upgrade() {
                    let r = yo.con(|c| cola::quitar_terminados(c));
                    yo.tras_cambio(r);
                }
            });
        }
        // Al abrir, lo que hubiera en la cola de antes (y el gestor, si tiene trabajo).
        yo.refrescar();
        let debil = Rc::downgrade(&yo);
        glib::timeout_add_local(std::time::Duration::from_millis(500), move || match debil.upgrade() {
            Some(yo) => {
                yo.refrescar();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        yo
    }

    /// Con la conexión a la cola (se abre la primera vez).
    fn con<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T, String>) -> Result<T, String> {
        let mut con = self.con.borrow_mut();
        if con.is_none() {
            *con = Some(cola::abrir()?);
        }
        f(con.as_mut().expect("conexión"))
    }

    fn tras_cambio(self: &Rc<Self>, r: Result<(), String>) {
        if let Err(e) = r {
            self.ctx.aviso(&e);
        }
        self.refrescar();
    }

    /// Descarga de una web nativa: a la cola (si no está ya).
    pub fn descargar(self: &Rc<Self>, titulo: &str, url: &str, pagina: &str, usar_al_terminar: bool) {
        let ajustes = self.ajustes_importar(self.ctx.optimizar_descargas());
        let r = self.con(|c| {
            cola::encolar(
                c,
                &cola::Nuevo {
                    clave: pagina,
                    nombre: titulo,
                    titulo: Some(titulo),
                    url: Some(url),
                    referer: Some(pagina),
                    fichero: None,
                    mover: true,
                    usar: usar_al_terminar,
                    ajustes: &ajustes,
                },
            )
        });
        self.avisar_encolado(titulo, r);
    }

    /// Un fichero a importar: de los que elige el usuario (se copia) o bajado por el navegador
    /// (se mueve).
    fn encolar_fichero(self: &Rc<Self>, ruta: &Path, nombre: &str, mover: bool, optimizar: Option<(i32, i32)>) {
        let ajustes = self.ajustes_importar(optimizar);
        let clave = ruta.to_string_lossy();
        let r = self.con(|c| {
            cola::encolar(
                c,
                &cola::Nuevo {
                    clave: &clave,
                    nombre,
                    titulo: None,
                    url: None,
                    referer: None,
                    fichero: Some(ruta),
                    mover,
                    usar: false,
                    ajustes: &ajustes,
                },
            )
        });
        self.avisar_encolado(nombre, r);
    }

    fn ajustes_importar(&self, optimizar: Option<(i32, i32)>) -> cola::AjustesImportar {
        cola::AjustesImportar {
            biblioteca: self.ctx.biblioteca(),
            sin_audio: self.ctx.ajustes.boolean("strip-audio"),
            optimizar,
            version_30: self.ctx.version_30_al_importar(),
        }
    }

    fn avisar_encolado(self: &Rc<Self>, nombre: &str, r: Result<Encolado, String>) {
        match r {
            Ok(Encolado::Nuevo) => self.ctx.aviso(&format!("«{nombre}» añadido a la cola de descargas")),
            Ok(Encolado::Reintentado) => self.ctx.aviso(&format!("«{nombre}» ya estaba en la cola: se vuelve a intentar")),
            Ok(Encolado::YaEstaba) => self.ctx.aviso(&format!("«{nombre}» ya está en la cola")),
            Err(e) => self.ctx.aviso(&e),
        }
        self.refrescar();
    }

    /// Lee la cola y pone la lista al día; avisa de lo que ha terminado y arranca el gestor si
    /// hay trabajo y no está en marcha (no ha arrancado aún, o se ha caído).
    fn refrescar(self: &Rc<Self>) {
        let trabajos = match self.con(|c| {
            cola::olvidar_listas(c, QUEDARSE_TRAS_TERMINAR)?;
            cola::listar(c)
        }) {
            Ok(t) => t,
            Err(e) => {
                crate::depurar!("{e}");
                return;
            }
        };
        if trabajos.iter().any(|t| !t.terminado() && t.estado != cola::PAUSADA) {
            gestor::asegurar_en_marcha();
        }

        let mut filas = self.filas.borrow_mut();
        filas.retain(|id, f| {
            let sigue = trabajos.iter().any(|t| t.id == *id);
            if !sigue {
                self.lista.remove(&f.fila);
            }
            sigue
        });
        let n = trabajos.len();
        for (i, t) in trabajos.iter().enumerate() {
            let f = filas.entry(t.id).or_insert_with(|| self.fila_nueva(t.id));
            // Mismo orden que la cola.
            if self.lista.row_at_index(i as i32).as_ref() != Some(&f.fila) {
                if f.fila.parent().is_some() {
                    self.lista.remove(&f.fila);
                }
                self.lista.insert(&f.fila, i as i32);
            }
            pintar(f, t, i, n);
        }
        drop(filas);

        for t in trabajos.iter().filter(|t| t.avisada == 0 && (t.estado == cola::LISTA || t.estado == cola::ERROR)) {
            if let Err(e) = self.con(|c| cola::marcar_avisada(c, t.id)) {
                crate::depurar!("{e}");
            }
            if t.estado == cola::LISTA {
                self.ctx.biblioteca_cambiada();
                if let Some(ruta) = &t.ruta {
                    self.ctx.aviso_importado(ruta, &t.avisos);
                }
            } else {
                let e = t.mensaje.as_deref().unwrap_or("error");
                self.ctx.aviso(&format!("No se pudo traer «{}»: {e}", t.nombre));
            }
        }

        let pendientes = trabajos.iter().filter(|t| !t.terminado()).count() as u32 + self.del_navegador.get();
        self.lista.set_visible(n > 0);
        self.limpiar.set_sensitive(trabajos.iter().any(Trabajo::terminado));
        self.boton.set_visible(n > 0 || self.lista_navegador.first_child().is_some());
        self.boton.set_tooltip_text(Some(&match pendientes {
            0 => "Descargas".to_string(),
            1 => "Descargas (1 pendiente)".to_string(),
            p => format!("Descargas ({p} pendientes)"),
        }));
    }

    fn fila_nueva(self: &Rc<Self>, id: i64) -> Fila {
        let nombre = etiqueta_nombre("");
        let barra = gtk::ProgressBar::new();
        let texto = etiqueta_estado();
        let subir = boton("go-up-symbolic", "Subir en la cola");
        let bajar = boton("go-down-symbolic", "Bajar en la cola");
        let pausar = boton("media-playback-pause-symbolic", "Pausar");
        let reanudar = boton("media-playback-start-symbolic", "Reanudar");
        let reintentar = boton("view-refresh-symbolic", "Reintentar");
        let cancelar = boton("media-playback-stop-symbolic", "Cancelar");
        let quitar = boton("window-close-symbolic", "Quitar de la lista");
        let caja = caja_fila(&nombre, &barra, &texto, &[&subir, &bajar, &pausar, &reanudar, &reintentar, &cancelar, &quitar]);
        let fila = gtk::ListBoxRow::builder().child(&caja).activatable(false).build();
        for (b, accion) in [
            (&subir, Accion::Subir),
            (&bajar, Accion::Bajar),
            (&pausar, Accion::Pausar),
            (&reanudar, Accion::Reanudar),
            (&reintentar, Accion::Reintentar),
            (&cancelar, Accion::Cancelar),
            (&quitar, Accion::Quitar),
        ] {
            let debil: Weak<Self> = Rc::downgrade(self);
            b.connect_clicked(move |_| {
                if let Some(yo) = debil.upgrade() {
                    yo.hacer(id, accion);
                }
            });
        }
        Fila { fila, nombre, barra, texto, subir, bajar, pausar, reanudar, reintentar, cancelar, quitar, ultimo: RefCell::default() }
    }

    fn hacer(self: &Rc<Self>, id: i64, accion: Accion) {
        let r = self.con(|c| match accion {
            Accion::Subir => cola::mover(c, id, -1),
            Accion::Bajar => cola::mover(c, id, 1),
            Accion::Pausar => cola::pausar(c, id),
            Accion::Reanudar => cola::reanudar(c, id),
            Accion::Reintentar => cola::reintentar(c, id).map_err(|e| format!("No se puede reintentar: {e}")),
            Accion::Cancelar => cola::cancelar(c, id),
            Accion::Quitar => cola::quitar(c, id),
        });
        self.tras_cambio(r);
    }

    /// Descarga hecha por el navegador integrado (WebKit): se sigue aquí y, al acabar, a la cola
    /// para importarla.
    pub fn seguir_webkit(self: &Rc<Self>, descarga: &webkit6::Download) {
        let dir = cola::carpeta();
        let _ = std::fs::create_dir_all(&dir);
        let nombre = etiqueta_nombre("Descarga del navegador");
        let barra = gtk::ProgressBar::new();
        let texto = etiqueta_estado();
        texto.set_label("Empezando…");
        let cancelar = boton("media-playback-stop-symbolic", "Cancelar");
        let fila = gtk::ListBoxRow::builder()
            .child(&caja_fila(&nombre, &barra, &texto, &[&cancelar]))
            .activatable(false)
            .build();
        self.lista_navegador.append(&fila);
        self.lista_navegador.set_visible(true);
        self.del_navegador.set(self.del_navegador.get() + 1);
        self.refrescar();
        {
            let d = descarga.clone();
            cancelar.connect_clicked(move |_| d.cancel());
        }
        {
            let nombre = nombre.clone();
            descarga.connect_decide_destination(move |d, sugerido| {
                nombre.set_label(sugerido);
                nombre.set_tooltip_text(Some(sugerido));
                let (base, ext) = sugerido.rsplit_once('.').unwrap_or((sugerido, "bin"));
                let destino = config::ruta_libre(&dir, &config::nombre_limpio(base), ext);
                d.set_destination(&destino.to_string_lossy());
                true
            });
        }
        {
            let barra = barra.clone();
            let texto = texto.clone();
            descarga.connect_estimated_progress_notify(move |d| {
                barra.set_fraction(d.estimated_progress());
                texto.set_label(&format!("{} descargado", config::tamano_legible(d.received_data_length())));
            });
        }
        let fallo = Rc::new(Cell::new(false));
        {
            let fallo = fallo.clone();
            let texto = texto.clone();
            descarga.connect_failed(move |_, error| {
                fallo.set(true);
                texto.set_label(&format!("Error: {error}"));
            });
        }
        let debil = Rc::downgrade(self);
        descarga.connect_finished(move |d| {
            let Some(yo) = debil.upgrade() else { return };
            yo.del_navegador.set(yo.del_navegador.get().saturating_sub(1));
            cancelar.set_visible(false);
            let destino = d.destination().map(|s| PathBuf::from(s.as_str()));
            match destino {
                Some(fichero) if !fallo.get() => {
                    // Ya en la cola: esta fila sobra.
                    yo.lista_navegador.remove(&fila);
                    let nombre = nombre.label().to_string();
                    yo.encolar_fichero(&fichero, &nombre, true, yo.ctx.optimizar_descargas());
                }
                _ => {
                    if let Some(f) = destino {
                        let _ = std::fs::remove_file(f);
                    }
                    let yo2 = yo.clone();
                    let fila = fila.clone();
                    glib::timeout_add_seconds_local_once(QUEDARSE_TRAS_TERMINAR as u32, move || {
                        yo2.lista_navegador.remove(&fila);
                        yo2.refrescar();
                    });
                }
            }
            yo.lista_navegador.set_visible(yo.lista_navegador.first_child().is_some());
            yo.refrescar();
        });
    }

    /// Importar ficheros o carpetas elegidos por el usuario (o soltados en la ventana).
    pub fn importar_locales(self: &Rc<Self>, rutas: Vec<PathBuf>) {
        for ruta in rutas {
            let nombre = ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.encolar_fichero(&ruta, &nombre, false, None);
        }
    }

    /// Descargas que se pierden si se cierra la ventana: las del navegador (las de la cola las
    /// sigue el gestor).
    pub fn hay_activas(&self) -> bool {
        self.del_navegador.get() > 0
    }
}

/// Pone una fila al día (solo si el trabajo ha cambiado).
fn pintar(f: &Fila, t: &Trabajo, i: usize, n: usize) {
    if t.estado == cola::DESCARGANDO && t.total.is_none() || t.estado == cola::IMPORTANDO {
        f.barra.pulse();
    }
    if f.ultimo.borrow().as_ref() == Some(t) {
        f.subir.set_sensitive(i > 0);
        f.bajar.set_sensitive(i + 1 < n);
        return;
    }
    f.nombre.set_label(&t.nombre);
    f.nombre.set_tooltip_text(Some(&t.nombre));
    f.texto.set_label(&describir(t));
    match (t.estado.as_str(), t.total) {
        (cola::LISTA, _) => f.barra.set_fraction(1.0),
        (_, Some(total)) if total > 0 => f.barra.set_fraction(t.hecho as f64 / total as f64),
        (cola::POR_IMPORTAR, _) => f.barra.set_fraction(1.0),
        (cola::DESCARGANDO | cola::IMPORTANDO, _) => {}
        _ => f.barra.set_fraction(0.0),
    }
    let e = t.estado.as_str();
    let descarga = t.url.is_some();
    let en_marcha_sin_peticion = e == cola::DESCARGANDO && t.peticion.is_none();
    let en_espera = matches!(e, cola::EN_COLA | cola::PAUSADA | cola::POR_IMPORTAR);
    f.subir.set_visible(en_espera);
    f.bajar.set_visible(en_espera);
    f.subir.set_sensitive(i > 0);
    f.bajar.set_sensitive(i + 1 < n);
    f.pausar.set_visible(descarga && (e == cola::EN_COLA || en_marcha_sin_peticion));
    f.reanudar.set_visible(e == cola::PAUSADA);
    f.reintentar.set_visible(e == cola::ERROR || e == cola::CANCELADA);
    f.cancelar.set_visible(en_espera || en_marcha_sin_peticion);
    f.quitar.set_visible(t.terminado());
    f.ultimo.replace(Some(t.clone()));
}
