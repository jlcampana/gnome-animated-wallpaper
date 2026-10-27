//! Base de datos de la biblioteca (SQLite, en ~/.local/share/fondos-animados/biblioteca.db): lo
//! que se sabe de cada vídeo (fps, resolución, códec…), para no lanzar ffprobe en cada recarga.
//! Una fila sirve mientras el fichero no cambie de tamaño ni de fecha. Guarda también de dónde
//! se bajó cada vídeo (tabla «origenes»), para exportar la biblioteca, y los favoritos de
//! Explorar (tabla «favoritos»). Todo bloqueante: se llama
//! desde gio::spawn_blocking (o desde el gestor de descargas).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::config;
use crate::fuentes::Resultado;
use crate::importar::{self, InfoVideo};

fn abrir() -> rusqlite::Result<Connection> {
    let dir = config::carpeta_datos();
    let _ = std::fs::create_dir_all(&dir);
    abrir_en(&dir.join("biblioteca.db"))
}

fn abrir_en(ruta: &Path) -> rusqlite::Result<Connection> {
    let con = Connection::open(ruta)?;
    con.busy_timeout(Duration::from_secs(5))?;
    con.execute_batch(
        "CREATE TABLE IF NOT EXISTS videos (
            ruta       TEXT PRIMARY KEY,
            tamano     INTEGER NOT NULL,
            modificado INTEGER NOT NULL,
            ancho      INTEGER NOT NULL,
            alto       INTEGER NOT NULL,
            fps        REAL NOT NULL,
            duracion   REAL NOT NULL,
            codec      TEXT NOT NULL,
            audio      INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS origenes (
            ruta     TEXT PRIMARY KEY,
            pagina   TEXT NOT NULL,
            descarga TEXT,
            titulo   TEXT
        );
        -- Vídeos sin origen apuntado que ya se buscaron en las webs sin dar con ellos.
        CREATE TABLE IF NOT EXISTS sin_origen (ruta TEXT PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS favoritos (
            pagina     TEXT PRIMARY KEY,
            web        TEXT NOT NULL,
            titulo     TEXT NOT NULL,
            miniatura  TEXT NOT NULL,
            resolucion TEXT,
            anadido    INTEGER NOT NULL
        );",
    )?;
    Ok(con)
}

/// Tamaño y fecha de modificación (en ns), lo que dice si una fila sigue valiendo.
fn huella(ruta: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(ruta).ok()?;
    let fecha = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as i64;
    Some((m.len(), fecha))
}

fn leer(con: &Connection, ruta: &str, tamano: u64, fecha: i64) -> rusqlite::Result<Option<InfoVideo>> {
    con.query_row(
        "SELECT ancho, alto, fps, duracion, codec, audio FROM videos
         WHERE ruta = ?1 AND tamano = ?2 AND modificado = ?3",
        params![ruta, tamano as i64, fecha],
        |f| {
            Ok(InfoVideo {
                ancho: f.get(0)?,
                alto: f.get(1)?,
                fps: f.get(2)?,
                duracion: f.get(3)?,
                codec: f.get(4)?,
                audio: f.get(5)?,
                tamano,
            })
        },
    )
    .optional()
}

fn guardar(con: &Connection, ruta: &str, fecha: i64, i: &InfoVideo) -> rusqlite::Result<()> {
    con.execute(
        "INSERT OR REPLACE INTO videos (ruta, tamano, modificado, ancho, alto, fps, duracion, codec, audio)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![ruta, i.tamano as i64, fecha, i.ancho, i.alto, i.fps, i.duracion, i.codec, i.audio],
    )?;
    Ok(())
}

/// La información de `rutas` (de la base de datos o, si el fichero es nuevo o ha cambiado, con
/// ffprobe), y fuera las filas de vídeos que ya no están. Los que ffprobe no entiende no salen.
pub fn sincronizar(rutas: &[PathBuf]) -> HashMap<PathBuf, InfoVideo> {
    match abrir() {
        Ok(con) => sincronizar_en(&con, rutas),
        Err(e) => {
            // Sin base de datos se puede seguir, solo que más lento.
            crate::depurar!("base de datos: {e}");
            rutas.iter().filter_map(|r| Some((r.clone(), importar::info(r).ok()?))).collect()
        }
    }
}

fn sincronizar_en(con: &Connection, rutas: &[PathBuf]) -> HashMap<PathBuf, InfoVideo> {
    let mut infos = HashMap::new();
    for ruta in rutas {
        let Some((tamano, fecha)) = huella(ruta) else { continue };
        let clave = ruta.to_string_lossy();
        if let Ok(Some(info)) = leer(con, &clave, tamano, fecha) {
            infos.insert(ruta.clone(), info);
            continue;
        }
        let Ok(info) = importar::info(ruta) else { continue };
        if let Err(e) = guardar(con, &clave, fecha, &info) {
            crate::depurar!("base de datos: no se pudo guardar {clave}: {e}");
        }
        infos.insert(ruta.clone(), info);
    }

    let presentes: HashSet<String> = infos.keys().map(|r| r.to_string_lossy().into_owned()).collect();
    let viejas: Vec<String> = con
        .prepare("SELECT ruta FROM videos")
        .and_then(|mut c| c.query_map([], |f| f.get(0))?.collect())
        .unwrap_or_default();
    for ruta in viejas.into_iter().filter(|r| !presentes.contains(r)) {
        let _ = con.execute("DELETE FROM videos WHERE ruta = ?1", [&ruta]);
    }
    infos
}

/// De dónde salió un vídeo de la biblioteca.
#[derive(Clone, Debug, PartialEq)]
pub struct Origen {
    /// La página del vídeo en la web.
    pub pagina: String,
    /// La dirección de descarga (None: se bajó con el navegador integrado).
    pub descarga: Option<String>,
    pub titulo: Option<String>,
}

pub fn guardar_origen(ruta: &Path, o: &Origen) -> Result<(), String> {
    guardar_origen_en(&abrir().map_err(|e| e.to_string())?, ruta, o).map_err(|e| e.to_string())
}

fn guardar_origen_en(con: &Connection, ruta: &Path, o: &Origen) -> rusqlite::Result<()> {
    let ruta = ruta.to_string_lossy();
    con.execute(
        "INSERT OR REPLACE INTO origenes (ruta, pagina, descarga, titulo) VALUES (?1, ?2, ?3, ?4)",
        params![ruta, o.pagina, o.descarga, o.titulo],
    )?;
    con.execute("DELETE FROM sin_origen WHERE ruta = ?1", [&ruta])?;
    Ok(())
}

/// Se buscó el origen de `ruta` y no se encontró: no volver a buscarlo.
pub fn marcar_sin_origen(ruta: &Path) -> Result<(), String> {
    let con = abrir().map_err(|e| e.to_string())?;
    con.execute("INSERT OR IGNORE INTO sin_origen (ruta) VALUES (?1)", [ruta.to_string_lossy()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Los vídeos ya buscados sin éxito.
pub fn sin_origen() -> HashSet<PathBuf> {
    abrir()
        .and_then(|con| {
            let mut c = con.prepare("SELECT ruta FROM sin_origen")?;
            c.query_map([], |f| Ok(PathBuf::from(f.get::<_, String>(0)?)))?.collect()
        })
        .unwrap_or_default()
}

/// El vídeo ha cambiado de sitio (una conversión que sustituye al original): su origen, con él.
pub fn mover_origen(de: &Path, a: &Path) -> Result<(), String> {
    if de == a {
        return Ok(());
    }
    let con = abrir().map_err(|e| e.to_string())?;
    con.execute(
        "UPDATE OR REPLACE origenes SET ruta = ?1 WHERE ruta = ?2",
        params![a.to_string_lossy(), de.to_string_lossy()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Todos los orígenes conocidos (también de vídeos que ya no estén: se filtra al usarlos, y así
/// no se pierden si la biblioteca cambia de carpeta un rato).
pub fn origenes() -> HashMap<PathBuf, Origen> {
    abrir().and_then(|con| origenes_en(&con)).unwrap_or_default()
}

fn origenes_en(con: &Connection) -> rusqlite::Result<HashMap<PathBuf, Origen>> {
    let mut c = con.prepare("SELECT ruta, pagina, descarga, titulo FROM origenes")?;
    c.query_map([], |f| {
        Ok((
            PathBuf::from(f.get::<_, String>(0)?),
            Origen { pagina: f.get(1)?, descarga: f.get(2)?, titulo: f.get(3)? },
        ))
    })?
    .collect()
}

// ── Favoritos de Explorar ──

/// Los favoritos, el último marcado primero: (web de la fuente, resultado).
pub fn favoritos() -> Vec<(String, Resultado)> {
    abrir().and_then(|con| favoritos_en(&con)).unwrap_or_default()
}

fn favoritos_en(con: &Connection) -> rusqlite::Result<Vec<(String, Resultado)>> {
    let mut c = con.prepare("SELECT web, pagina, titulo, miniatura, resolucion FROM favoritos ORDER BY anadido DESC, rowid DESC")?;
    c.query_map([], |f| {
        Ok((f.get(0)?, Resultado { pagina: f.get(1)?, titulo: f.get(2)?, miniatura: f.get(3)?, resolucion: f.get(4)? }))
    })?
    .collect()
}

/// Marca (o desmarca) un resultado de `web` como favorito.
pub fn poner_favorito(web: &str, r: &Resultado, favorito: bool) -> Result<(), String> {
    poner_favorito_en(&abrir().map_err(|e| e.to_string())?, web, r, favorito).map_err(|e| e.to_string())
}

fn poner_favorito_en(con: &Connection, web: &str, r: &Resultado, favorito: bool) -> rusqlite::Result<()> {
    if favorito {
        let ahora = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        con.execute(
            "INSERT OR REPLACE INTO favoritos (pagina, web, titulo, miniatura, resolucion, anadido)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![r.pagina, web, r.titulo, r.miniatura, r.resolucion, ahora],
        )?;
    } else {
        con.execute("DELETE FROM favoritos WHERE pagina = ?1", [&r.pagina])?;
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::process::Command;

    #[test]
    fn guarda_reutiliza_y_olvida() {
        let dir = std::env::temp_dir().join(format!("fondos-prueba-bd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let video = dir.join("rapido.mp4");
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=320x180:r=60:d=1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(ok.success());
        let con = abrir_en(&dir.join("prueba.db")).unwrap();
        let rutas = vec![video.clone()];

        let infos = sincronizar_en(&con, &rutas);
        assert_eq!(infos[&video].fps.round(), 60.0);
        assert_eq!((infos[&video].ancho, infos[&video].alto), (320, 180));

        // La segunda vez sale de la base de datos, no de ffprobe.
        con.execute("UPDATE videos SET fps = 12.5", []).unwrap();
        assert_eq!(sincronizar_en(&con, &rutas)[&video].fps, 12.5);

        // Si el fichero cambia, se vuelve a leer.
        let mut f = std::fs::OpenOptions::new().append(true).open(&video).unwrap();
        std::io::Write::write_all(&mut f, &[0; 16]).unwrap();
        drop(f);
        assert_eq!(sincronizar_en(&con, &rutas)[&video].fps.round(), 60.0);

        // Y si desaparece, su fila también.
        std::fs::remove_file(&video).unwrap();
        assert!(sincronizar_en(&con, &rutas).is_empty());
        let filas: i64 = con.query_row("SELECT COUNT(*) FROM videos", [], |f| f.get(0)).unwrap();
        assert_eq!(filas, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn origenes_guardados() {
        let dir = std::env::temp_dir().join(format!("fondos-prueba-origenes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let con = abrir_en(&dir.join("prueba.db")).unwrap();
        let o = Origen { pagina: "https://x/p/".into(), descarga: Some("https://x/dl".into()), titulo: Some("X".into()) };
        guardar_origen_en(&con, Path::new("/b/x.mp4"), &o).unwrap();
        // La segunda vez sustituye a la primera.
        guardar_origen_en(&con, Path::new("/b/x.mp4"), &o).unwrap();
        assert_eq!(origenes_en(&con).unwrap(), HashMap::from([(PathBuf::from("/b/x.mp4"), o)]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn favoritos_guardados() {
        let dir = std::env::temp_dir().join(format!("fondos-prueba-favoritos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let con = abrir_en(&dir.join("prueba.db")).unwrap();
        let r = |p: &str| Resultado { titulo: p.into(), pagina: p.into(), miniatura: "m".into(), resolucion: None };
        poner_favorito_en(&con, "https://a", &r("1"), true).unwrap();
        poner_favorito_en(&con, "https://b", &r("2"), true).unwrap();
        poner_favorito_en(&con, "https://b", &r("2"), true).unwrap(); // repetido: no se duplica
        let paginas = |con: &Connection| favoritos_en(con).unwrap().into_iter().map(|(_, r)| r.pagina).collect::<Vec<_>>();
        assert_eq!(paginas(&con), ["2", "1"]);
        poner_favorito_en(&con, "https://a", &r("1"), false).unwrap();
        assert_eq!(paginas(&con), ["2"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
