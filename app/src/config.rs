//! Rutas, ajustes compartidos con la extensión y localización de los scripts.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use gtk::{gio, glib, prelude::*};

pub const ID_APP: &str = "io.github.jlcampana.FondosAnimados";
pub const ESQUEMA: &str = "org.gnome.shell.extensions.animated-wallpaper";
pub const UUID_EXTENSION: &str = "animated-wallpaper@jlcampana.github.io";
pub const AGENTE: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";
pub const EXTENSIONES_VIDEO: &[&str] = &["mp4", "webm", "mkv", "mov", "m4v"];

static DEPURAR: AtomicBool = AtomicBool::new(false);

/// Registro de depuración: con FONDOS_ANIMADOS_DEBUG=1 o con la clave «debug» de los ajustes.
pub fn activar_depuracion(si: bool) {
    DEPURAR.store(si, Ordering::Relaxed);
}

pub fn depurando() -> bool {
    DEPURAR.load(Ordering::Relaxed)
}

/// Escribe en stderr solo si se está depurando.
#[macro_export]
macro_rules! depurar {
    ($($arg:tt)*) => {
        if $crate::config::depurando() {
            eprintln!("[fondos-animados] {}", format!($($arg)*));
        }
    };
}

/// Los ajustes de la extensión. None si el esquema no está instalado.
pub fn ajustes() -> Option<gio::Settings> {
    let fuente = gio::SettingsSchemaSource::default()?;
    fuente.lookup(ESQUEMA, true)?;
    Some(gio::Settings::new(ESQUEMA))
}

pub fn carpeta_biblioteca(ajustes: &gio::Settings) -> PathBuf {
    let dir = ajustes.string("library-dir");
    if !dir.is_empty() {
        return PathBuf::from(dir.as_str());
    }
    let videos = glib::user_special_dir(glib::UserDirectory::Videos)
        .unwrap_or_else(|| glib::home_dir().join("Videos"));
    videos.join("Fondos animados")
}

pub fn carpeta_cache() -> PathBuf {
    glib::user_cache_dir().join("fondos-animados")
}

pub fn carpeta_datos() -> PathBuf {
    glib::user_data_dir().join("fondos-animados")
}

/// Subcarpeta de la biblioteca con las versiones de 30 fps: mismo nombre que el original, en .mp4.
pub const CARPETA_30: &str = "30 fps";

/// Dónde está (o irá) la versión de 30 fps de un vídeo de la biblioteca.
pub fn ruta_version_30(original: &Path) -> PathBuf {
    let base = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    original.parent().unwrap_or(Path::new(".")).join(CARPETA_30).join(format!("{base}.mp4"))
}

/// Ficheros a medias de una descarga o una conversión.
pub fn es_temporal(ruta: &Path) -> bool {
    let nombre = ruta.to_string_lossy();
    [".tmp.", ".convirtiendo.", ".optimizando."].iter().any(|t| nombre.contains(t))
}

pub fn es_video(ruta: &Path) -> bool {
    ruta.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONES_VIDEO.contains(&e.to_lowercase().as_str()))
}

/// Un script o ejecutable del proyecto: junto a este ejecutable, en ~/.local/bin, ~/scripts o
/// en el PATH.
pub fn programa(nombre: &str) -> PathBuf {
    let mut sitios = vec![];
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        sitios.push(dir.clone());
        // En desarrollo (target/release) los scripts están en ../../scripts.
        sitios.push(dir.join("../../scripts"));
    }
    sitios.push(glib::home_dir().join(".local/bin"));
    sitios.push(glib::home_dir().join("scripts"));
    // Compilación de desarrollo (y tests): la carpeta scripts del repositorio.
    if cfg!(debug_assertions) {
        sitios.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts"));
    }
    for dir in sitios {
        let ruta = dir.join(nombre);
        if ruta.is_file() {
            return ruta;
        }
    }
    glib::find_program_in_path(nombre).unwrap_or_else(|| PathBuf::from(nombre))
}

/// Resolución física del monitor más grande (la que conviene a los fondos).
pub fn resolucion_pantalla() -> (i32, i32) {
    let mut mejor = (1920, 1080);
    if let Some(display) = gtk::gdk::Display::default() {
        let monitores = display.monitors();
        for i in 0..monitores.n_items() {
            let Some(m) = monitores.item(i).and_downcast::<gtk::gdk::Monitor>() else { continue };
            let g = m.geometry();
            let escala = m.scale();
            let (w, h) = ((g.width() as f64 * escala).round() as i32, (g.height() as f64 * escala).round() as i32);
            if w * h > mejor.0 * mejor.1 {
                mejor = (w, h);
            }
        }
    }
    mejor
}

/// Nombre de fichero limpio a partir de un título.
pub fn nombre_limpio(titulo: &str) -> String {
    let mut s = String::new();
    let mut guion = false;
    for c in titulo.trim().chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
            guion = false;
        } else if !guion && !s.is_empty() {
            s.push('-');
            guion = true;
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "fondo".into() } else { s.chars().take(80).collect() }
}

/// Ruta libre en `dir` para `base.ext` (añade -2, -3… si ya existe).
pub fn ruta_libre(dir: &Path, base: &str, ext: &str) -> PathBuf {
    let mut ruta = dir.join(format!("{base}.{ext}"));
    let mut n = 2;
    while ruta.exists() {
        ruta = dir.join(format!("{base}-{n}.{ext}"));
        n += 1;
    }
    ruta
}

pub fn tamano_legible(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", b / 1024f64.powi(3))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / 1024f64.powi(2))
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn nombres() {
        assert_eq!(nombre_limpio("Yamada & Tayama Live Wallpaper"), "yamada-tayama-live-wallpaper");
        assert_eq!(nombre_limpio("  ¡Frieren!  "), "frieren");
        assert_eq!(nombre_limpio("***"), "fondo");
    }
}
