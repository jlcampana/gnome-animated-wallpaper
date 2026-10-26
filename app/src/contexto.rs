//! Lo que comparten todas las páginas: ajustes, ventana, avisos y miniaturas.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::{config, importar};

pub struct Contexto {
    pub ajustes: gio::Settings,
    pub ventana: adw::ApplicationWindow,
    pub avisos: adw::ToastOverlay,
    /// Se avisa a quien escuche (la biblioteca) cuando entra un vídeo nuevo.
    al_cambiar_biblioteca: RefCell<Vec<Box<dyn Fn()>>>,
    /// Ir a la página del navegador con una URL.
    pub abrir_navegador: RefCell<Option<Box<dyn Fn(&str)>>>,
    /// Mirar si hay una versión nueva (true: comprobación pedida a mano, avisa también si no hay).
    pub buscar_actualizacion: RefCell<Option<Box<dyn Fn(bool)>>>,
    /// Hardware de vídeo de este equipo (se detecta al arrancar, en segundo plano).
    pub capacidades: RefCell<Option<importar::Capacidades>>,
    al_detectar: RefCell<Vec<Box<dyn Fn(&importar::Capacidades)>>>,
}

impl Contexto {
    pub fn new(ajustes: gio::Settings, ventana: adw::ApplicationWindow, avisos: adw::ToastOverlay) -> Rc<Self> {
        Rc::new(Self {
            ajustes,
            ventana,
            avisos,
            al_cambiar_biblioteca: RefCell::default(),
            abrir_navegador: RefCell::default(),
            buscar_actualizacion: RefCell::default(),
            capacidades: RefCell::default(),
            al_detectar: RefCell::default(),
        })
    }

    /// Detecta (o vuelve a detectar) el hardware de vídeo.
    pub fn detectar_capacidades(self: &Rc<Self>, forzar: bool) {
        let ctx = self.clone();
        glib::spawn_future_local(async move {
            let cap = gio::spawn_blocking(move || importar::capacidades(forzar)).await.unwrap_or_default();
            for f in ctx.al_detectar.borrow().iter() {
                f(&cap);
            }
            ctx.capacidades.replace(Some(cap));
        });
    }

    pub fn conectar_capacidades(&self, f: impl Fn(&importar::Capacidades) + 'static) {
        if let Some(cap) = self.capacidades.borrow().as_ref() {
            f(cap);
        }
        self.al_detectar.borrow_mut().push(Box::new(f));
    }

    /// Resolución a la que adaptar las descargas, según el ajuste optimize-downloads.
    pub fn optimizar_descargas(&self) -> Option<(i32, i32)> {
        let activo = match self.ajustes.string("optimize-downloads").as_str() {
            "always" => true,
            "never" => false,
            _ => self.capacidades.borrow().as_ref().is_some_and(|c| c.codifica_por_gpu()),
        };
        activo.then(config::resolucion_pantalla)
    }

    /// ¿Toca la versión de 30 fps? La preferida, o la de 30 si se va con batería y está
    /// battery-30fps (lo mismo que decide la extensión).
    pub fn preferir_30(&self) -> bool {
        self.ajustes.string("preferred-version") == "30fps"
            || (self.ajustes.boolean("battery-30fps") && config::con_bateria())
    }

    /// Calidad (QP/CRF) de las versiones de 30 fps (ajuste quality-30fps).
    pub fn calidad_30(&self) -> i32 {
        self.ajustes.int("quality-30fps")
    }

    /// Si al importar hay que crear también la versión de 30 fps, con qué calidad.
    pub fn version_30_al_importar(&self) -> Option<i32> {
        self.ajustes.boolean("create-30fps").then(|| self.calidad_30())
    }

    pub fn aviso(&self, texto: &str) {
        // Sin markup: los títulos de los vídeos pueden llevar «&» y similares.
        self.avisos.add_toast(adw::Toast::builder().title(texto).use_markup(false).build());
    }

    pub fn biblioteca(&self) -> PathBuf {
        config::carpeta_biblioteca(&self.ajustes)
    }

    pub fn usar_video(&self, ruta: &Path) {
        let _ = self.ajustes.set_string("video", &ruta.to_string_lossy());
        let _ = self.ajustes.set_boolean("enabled", true);
    }

    pub fn video_actual(&self) -> PathBuf {
        PathBuf::from(self.ajustes.string("video").as_str())
    }

    pub fn conectar_biblioteca(&self, f: impl Fn() + 'static) {
        self.al_cambiar_biblioteca.borrow_mut().push(Box::new(f));
    }

    pub fn biblioteca_cambiada(&self) {
        for f in self.al_cambiar_biblioteca.borrow().iter() {
            f();
        }
    }

    pub fn navegar(&self, url: &str) {
        if let Some(f) = self.abrir_navegador.borrow().as_ref() {
            f(url);
        }
    }

    /// Aviso con un botón «Usar de fondo».
    pub fn aviso_importado(self: &Rc<Self>, ruta: &Path, avisos: &[String]) {
        let nombre = ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut texto = format!("«{nombre}» añadido a la biblioteca");
        if !avisos.is_empty() {
            texto.push_str(&format!(" ({})", avisos.join("; ")));
        }
        let toast = adw::Toast::builder().title(texto).use_markup(false).button_label("Usar de fondo").timeout(8).build();
        let ctx = self.clone();
        let ruta = ruta.to_path_buf();
        toast.connect_button_clicked(move |_| ctx.usar_video(&ruta));
        self.avisos.add_toast(toast);
    }
}

/// Miniatura de un vídeo local (caché en ~/.cache/fondos-animados/miniaturas). Bloqueante.
pub fn miniatura_local(video: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(video).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let clave = glib::compute_checksum_for_string(
        glib::ChecksumType::Md5,
        &format!("{}|{}|{}", video.display(), meta.len(), mtime),
    )?;
    let ruta = config::carpeta_cache().join("miniaturas").join(format!("{clave}.jpg"));
    if ruta.is_file() {
        return Some(ruta);
    }
    let ok = std::process::Command::new(config::programa("fondo-animado-miniatura"))
        .arg(video)
        .arg(&ruta)
        .output()
        .is_ok_and(|o| o.status.success());
    ok.then_some(ruta)
}

/// Pone en `imagen` la miniatura de un vídeo local, sin bloquear.
pub fn cargar_miniatura_local(imagen: &gtk::Picture, video: &Path) {
    let video = video.to_path_buf();
    let imagen = imagen.downgrade();
    glib::spawn_future_local(async move {
        let ruta = gio::spawn_blocking(move || miniatura_local(&video)).await.ok().flatten();
        if let (Some(ruta), Some(imagen)) = (ruta, imagen.upgrade()) {
            imagen.set_filename(Some(&ruta));
        }
    });
}

/// Pone en `imagen` una imagen remota (con caché en disco), sin bloquear.
pub fn cargar_miniatura_remota(imagen: &gtk::Picture, url: &str, referer: &str) {
    let url = url.to_string();
    let referer = referer.to_string();
    let imagen = imagen.downgrade();
    glib::spawn_future_local(async move {
        let bytes = gio::spawn_blocking(move || {
            let clave = glib::compute_checksum_for_string(glib::ChecksumType::Md5, &url)?;
            let ruta = config::carpeta_cache().join("remotas").join(clave.as_str());
            if let Ok(b) = std::fs::read(&ruta) {
                return Some(b);
            }
            let b = crate::red::bytes(&url, Some(&referer)).ok()?;
            let _ = std::fs::create_dir_all(ruta.parent()?);
            let _ = std::fs::write(&ruta, &b);
            Some(b)
        })
        .await
        .ok()
        .flatten();
        let (Some(bytes), Some(imagen)) = (bytes, imagen.upgrade()) else { return };
        if let Ok(textura) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) {
            imagen.set_paintable(Some(&textura));
        }
    });
}
