//! Gestor de descargas: el proceso `fondos-animados --gestor-descargas`, independiente de la
//! ventana (si la app se cierra o se cae, sigue). Saca el trabajo de la cola (cola.rs): baja las
//! descargas de una en una, en el orden de la cola, y las importa a la biblioteca, también de una
//! en una (mientras importa una, ya está bajando la siguiente). Un cerrojo sobre un fichero
//! impide que haya dos; cuando no queda nada que hacer, se cierra. La app lo arranca cuando hay
//! trabajo y no está en marcha.
//!
//! Instalado, va como servicio de systemd de usuario (fondos-animados-descargas.service):
//! arranca al iniciar sesión (sigue con la cola que quedara) y systemd lo vuelve a arrancar si
//! se cae. Sin el servicio (o en desarrollo), la app lo lanza directamente.

use std::cell::Cell;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use gtk::gio;
use gtk::prelude::*;

use crate::cola::{self, Trabajo};
use crate::{config, importar, red};

/// Segundos sin nada que hacer antes de cerrarse.
const ESPERA_ANTES_DE_SALIR: u64 = 10;

fn ruta_cerrojo() -> PathBuf {
    config::carpeta_datos().join("gestor-descargas.lock")
}

/// El cerrojo de «hay un gestor en marcha» (se suelta al cerrar el fichero o al morir).
fn cerrojo() -> Option<File> {
    let _ = std::fs::create_dir_all(config::carpeta_datos());
    let f = File::options().create(true).truncate(false).write(true).open(ruta_cerrojo()).ok()?;
    f.try_lock().ok()?;
    Some(f)
}

pub fn en_marcha() -> bool {
    let Ok(f) = File::options().create(true).truncate(false).write(true).open(ruta_cerrojo()) else {
        return false;
    };
    match f.try_lock() {
        Ok(()) => false, // libre: no hay gestor (se suelta al cerrar `f`)
        Err(_) => true,
    }
}

pub const SERVICIO: &str = "fondos-animados-descargas.service";

/// La unidad de systemd del gestor, si está instalada y lanza este mismo ejecutable (así una
/// compilación de desarrollo no arranca el gestor instalado).
fn servicio_instalado(exe: &Path) -> bool {
    let unidad = gtk::glib::user_config_dir().join("systemd/user").join(SERVICIO);
    let Ok(texto) = std::fs::read_to_string(unidad) else { return false };
    texto
        .lines()
        .filter_map(|l| l.strip_prefix("ExecStart="))
        .any(|l| l.split_whitespace().next().map(Path::new) == Some(exe))
}

/// Cuándo se intentó arrancar por última vez (para no repetirlo mientras arranca).
static ULTIMO_ARRANQUE: AtomicI64 = AtomicI64::new(0);

/// Arranca el gestor si no está en marcha: con systemd si el servicio está instalado y, si no
/// (o falla), directamente.
pub fn asegurar_en_marcha() {
    if en_marcha() {
        return;
    }
    let ahora = cola::ahora();
    if ahora - ULTIMO_ARRANQUE.swap(ahora, Ordering::Relaxed) < 3 {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    // En otro hilo: systemctl tarda un poco y esto se llama desde el de GTK.
    std::thread::spawn(move || {
        if servicio_instalado(&exe) {
            let ok = Command::new("systemctl")
                .args(["--user", "start", "--no-block", SERVICIO])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                crate::depurar!("gestor de descargas arrancado con systemd");
                return;
            }
        }
        lanzar(&exe);
    });
}

/// Lanza el gestor como proceso suelto, en su propio grupo de procesos (para que no le lleguen
/// las señales de la terminal de la app); su salida, a gestor-descargas.log.
fn lanzar(exe: &Path) {
    let _ = std::fs::create_dir_all(config::carpeta_cache());
    let log = File::options().create(true).append(true).open(config::carpeta_cache().join("gestor-descargas.log"));
    let mut c = Command::new(exe);
    c.arg("--gestor-descargas").stdin(Stdio::null()).stdout(Stdio::null());
    match log {
        Ok(f) => c.stderr(f),
        Err(_) => c.stderr(Stdio::null()),
    };
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    match c.spawn() {
        Ok(mut hijo) => {
            crate::depurar!("gestor de descargas arrancado (pid {})", hijo.id());
            // Que no se quede zombi si acaba mientras la app sigue abierta.
            let _ = hijo.wait();
        }
        Err(e) => eprintln!("[fondos-animados] no se pudo arrancar el gestor de descargas: {e}"),
    }
}

macro_rules! registro {
    ($($arg:tt)*) => {
        eprintln!("[gestor-descargas {}] {}", cola::ahora(), format!($($arg)*))
    };
}

/// Un hilo de trabajo y el trabajo que lleva.
type Hilo = Option<(i64, JoinHandle<()>)>;

/// Si el hilo ha acabado, lo recoge; si acabó con un pánico, el trabajo pasa a error (si no, se
/// quedaría «descargando» para siempre).
fn recoger(con: &rusqlite::Connection, hilo: &mut Hilo) -> bool {
    match hilo.take() {
        Some((id, h)) if h.is_finished() => {
            if h.join().is_err() {
                registro!("fallo interno con el trabajo {id}");
                let _ = cola::fallo(con, id, "fallo interno del gestor de descargas");
            }
            true
        }
        Some(otro) => {
            *hilo = Some(otro);
            false
        }
        None => true,
    }
}

/// Bucle principal del proceso gestor. Sale con error solo si no puede trabajar (para que
/// systemd lo reintente).
pub fn ejecutar() -> ExitCode {
    let ajustes = config::ajustes();
    config::activar_depuracion(
        std::env::var_os("FONDOS_ANIMADOS_DEBUG").is_some() || ajustes.as_ref().is_some_and(|a| a.boolean("debug")),
    );
    let Some(mut cerrojo) = cerrojo() else {
        crate::depurar!("ya hay un gestor de descargas en marcha");
        return ExitCode::SUCCESS;
    };
    let con = match cola::abrir() {
        Ok(c) => c,
        Err(e) => {
            registro!("{e}");
            return ExitCode::FAILURE;
        }
    };
    registro!("en marcha (pid {})", std::process::id());
    if let Err(e) = cola::recuperar(&con) {
        registro!("{e}");
    }
    limpiar_temporales(&con);

    let mut descarga: Hilo = None;
    let mut importacion: Hilo = None;
    let mut ocioso = Instant::now();
    loop {
        if recoger(&con, &mut descarga) {
            descarga = match cola::reclamar(&con, cola::EN_COLA, cola::DESCARGANDO) {
                Ok(Some(t)) => Some((t.id, std::thread::spawn(move || descargar(t)))),
                Ok(None) => None,
                Err(e) => {
                    registro!("{e}");
                    None
                }
            };
        }
        if recoger(&con, &mut importacion) {
            importacion = match cola::reclamar(&con, cola::POR_IMPORTAR, cola::IMPORTANDO) {
                Ok(Some(t)) => Some((t.id, std::thread::spawn(move || importar(t)))),
                Ok(None) => None,
                Err(e) => {
                    registro!("{e}");
                    None
                }
            };
        }
        if descarga.is_some() || importacion.is_some() {
            ocioso = Instant::now();
        } else if ocioso.elapsed() > Duration::from_secs(ESPERA_ANTES_DE_SALIR) {
            // Se suelta el cerrojo y se mira otra vez: si la app ha encolado justo ahora (antes
            // de ver que había gestor), se sigue; si después, ya arrancará ella otro.
            drop(cerrojo);
            if !cola::hay_trabajo(&con).unwrap_or(false) {
                break;
            }
            match self::cerrojo() {
                Some(c) => cerrojo = c,
                None => break, // ya ha arrancado otro
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    // Que lleguen a dconf los cambios de ajustes (poner un fondo) antes de salir.
    drop(ajustes);
    gio::Settings::sync();
    registro!("sin trabajo: fin");
    ExitCode::SUCCESS
}

/// Fuera los .part que no son de ningún trabajo de la cola.
fn limpiar_temporales(con: &rusqlite::Connection) {
    let Ok(trabajos) = cola::listar(con) else { return };
    let validos: Vec<PathBuf> = trabajos.iter().map(Trabajo::temporal).collect();
    let Ok(dir) = std::fs::read_dir(cola::carpeta()) else { return };
    for e in dir.flatten() {
        let ruta = e.path();
        if ruta.extension().is_some_and(|x| x == "part") && !validos.contains(&ruta) {
            let _ = std::fs::remove_file(&ruta);
        }
    }
}

fn descargar(t: Trabajo) {
    let con = match cola::abrir() {
        Ok(c) => c,
        Err(e) => return registro!("{e}"),
    };
    let (Some(url), referer) = (t.url.as_deref(), t.referer.as_deref()) else {
        let _ = cola::fallo(&con, t.id, "sin dirección de descarga");
        return;
    };
    registro!("descargando «{}»", t.nombre);
    let _ = std::fs::create_dir_all(cola::carpeta());
    let temporal = t.temporal();
    let ultimo = Cell::new(Instant::now() - Duration::from_secs(1));
    let peticion: Cell<Option<String>> = Cell::new(None);
    let total_conocido = Cell::new(t.total);
    let r = red::descargar(
        url,
        referer,
        &temporal,
        &|hecho, total| {
            total_conocido.set(total);
            // Cada medio segundo: se apunta el progreso y se mira si la app pide algo.
            if ultimo.get().elapsed() >= Duration::from_millis(500) {
                ultimo.set(Instant::now());
                match cola::progreso(&con, t.id, hecho, total) {
                    Ok(p) => peticion.set(p),
                    Err(e) => registro!("{e}"),
                }
            }
        },
        &|| {
            let p = peticion.take();
            let parar = p.is_some();
            peticion.set(p);
            parar
        },
    );
    let resultado = match r {
        Ok(nombre) => terminar_descarga(&con, &t, &temporal, nombre.as_deref()),
        Err(e) => match peticion.take().as_deref() {
            Some(cola::PEDIR_PAUSA) => {
                registro!("«{}» en pausa", t.nombre);
                if let Ok(m) = std::fs::metadata(&temporal) {
                    let _ = cola::progreso(&con, t.id, m.len(), total_conocido.get());
                }
                cola::pasar(&con, t.id, &[cola::DESCARGANDO], cola::PAUSADA).map(|_| ())
            }
            Some(_) => {
                registro!("«{}» cancelada", t.nombre);
                let _ = std::fs::remove_file(&temporal);
                cola::pasar(&con, t.id, &[cola::DESCARGANDO], cola::CANCELADA).map(|_| ())
            }
            None => {
                registro!("«{}»: {e}", t.nombre);
                cola::fallo(&con, t.id, &e)
            }
        },
    };
    if let Err(e) = resultado {
        registro!("{e}");
    }
}

fn terminar_descarga(con: &rusqlite::Connection, t: &Trabajo, temporal: &Path, nombre: Option<&str>) -> Result<(), String> {
    // Con la extensión que dice el servidor (puede ser .zip); si no la dice (o se reanudó sin
    // ella), por el contenido.
    let ext = nombre
        .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_lowercase()))
        .unwrap_or_else(|| extension_por_contenido(temporal).into());
    let base = format!("{}-{}", t.id, config::nombre_limpio(t.titulo.as_deref().unwrap_or(&t.nombre)));
    let fichero = config::ruta_libre(&cola::carpeta(), &base, &ext);
    std::fs::rename(temporal, &fichero).map_err(|e| e.to_string())?;
    registro!("«{}» descargada", t.nombre);
    cola::descargada(con, t.id, &fichero)
}

fn extension_por_contenido(ruta: &Path) -> &'static str {
    let mut cabecera = [0u8; 8];
    let leidos = File::open(ruta).and_then(|mut f| std::io::Read::read(&mut f, &mut cabecera)).unwrap_or(0);
    match &cabecera[..leidos] {
        [b'P', b'K', 3, 4, ..] => "zip",
        [b'R', b'a', b'r', b'!', ..] => "rar",
        [b'7', b'z', 0xBC, 0xAF, ..] => "7z",
        [0x1A, 0x45, 0xDF, 0xA3, ..] => "webm",
        _ => "mp4",
    }
}

fn importar(t: Trabajo) {
    let con = match cola::abrir() {
        Ok(c) => c,
        Err(e) => return registro!("{e}"),
    };
    let Some(fichero) = t.fichero.clone() else {
        let _ = cola::fallo(&con, t.id, "no hay fichero que importar");
        return;
    };
    registro!("importando «{}»", t.nombre);
    let a = &t.ajustes;
    let r = importar::importar(&fichero, &a.biblioteca, t.titulo.as_deref(), a.sin_audio, t.mover, a.optimizar, a.version_30);
    let resultado = match r {
        Ok(r) => {
            registro!("«{}» → {}", t.nombre, r.ruta.display());
            if t.usar {
                if let Some(ajustes) = config::ajustes() {
                    let _ = ajustes.set_string("video", &r.ruta.to_string_lossy());
                    let _ = ajustes.set_boolean("enabled", true);
                    gio::Settings::sync();
                }
            }
            cola::importada(&con, t.id, &r.ruta, &r.info.resumen(), &r.avisos)
        }
        Err(e) => {
            registro!("«{}»: {e}", t.nombre);
            cola::fallo(&con, t.id, &e)
        }
    };
    if let Err(e) = resultado {
        registro!("{e}");
    }
}
