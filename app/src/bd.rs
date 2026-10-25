//! Base de datos de la biblioteca (SQLite, en ~/.local/share/fondos-animados/biblioteca.db): lo
//! que se sabe de cada vídeo (fps, resolución, códec…), para no lanzar ffprobe en cada recarga.
//! Una fila sirve mientras el fichero no cambie de tamaño ni de fecha. Todo bloqueante: se
//! llama desde gio::spawn_blocking.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::config;
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
        )",
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
}
