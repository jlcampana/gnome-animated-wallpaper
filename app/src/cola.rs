//! Cola de descargas, importaciones y conversiones (SQLite, en
//! ~/.local/share/fondos-animados/descargas.db).
//! La comparten la app, que encola y pide cosas (pausar, cancelar, reordenar…), y el gestor de
//! descargas (gestor.rs), un proceso aparte que hace el trabajo: si uno de los dos se cae, la
//! cola sigue ahí. Los cambios de estado se hacen con UPDATE … WHERE estado = …, para que la app
//! y el gestor no se pisen.
//!
//! Estados: en_cola → descargando → por_importar → importando → lista, más pausada, error y
//! cancelada. Los ficheros locales y los que baja el navegador integrado entran ya por_importar.
//! Las conversiones (versiones de 30 fps y el diálogo «Convertir») van por_convertir →
//! convirtiendo → lista: el origen está en «fichero» y el resultado irá a «ruta».

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::config;

pub const EN_COLA: &str = "en_cola";
pub const DESCARGANDO: &str = "descargando";
pub const PAUSADA: &str = "pausada";
pub const POR_IMPORTAR: &str = "por_importar";
pub const IMPORTANDO: &str = "importando";
pub const POR_CONVERTIR: &str = "por_convertir";
pub const CONVIRTIENDO: &str = "convirtiendo";
pub const LISTA: &str = "lista";
pub const ERROR: &str = "error";
pub const CANCELADA: &str = "cancelada";

/// Estados en los que un trabajo ya no hace nada (se puede quitar de la lista).
pub const TERMINADOS: [&str; 3] = [LISTA, ERROR, CANCELADA];

/// Lo que la app le pide al gestor sobre una descarga en marcha (columna «peticion»).
pub const PEDIR_PAUSA: &str = "pausar";
pub const PEDIR_CANCELACION: &str = "cancelar";

#[derive(Clone, Debug, PartialEq)]
pub struct Trabajo {
    pub id: i64,
    /// Página del vídeo (web) o ruta del fichero: con ella se detectan los repetidos.
    pub clave: String,
    pub nombre: String,
    /// Título para la biblioteca (None: el del fichero).
    pub titulo: Option<String>,
    /// Descarga de una web (con url); si no, un fichero ya en disco.
    pub url: Option<String>,
    pub referer: Option<String>,
    pub fichero: Option<PathBuf>,
    /// Mover el fichero a la biblioteca (descargas) o copiarlo (ficheros del usuario).
    pub mover: bool,
    pub usar: bool,
    pub estado: String,
    pub peticion: Option<String>,
    pub hecho: u64,
    pub total: Option<u64>,
    /// El error, o el resumen del vídeo importado.
    pub mensaje: Option<String>,
    pub ruta: Option<PathBuf>,
    pub avisos: Vec<String>,
    /// Cuándo avisó la app del resultado (0: todavía no).
    pub avisada: i64,
    pub ajustes: AjustesImportar,
    pub conversion: Option<Conversion>,
}

/// Qué conversión hacer con fondo-animado-convertir.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversion {
    pub tipo: TipoConversion,
    /// Opciones del script (sin --progreso, el origen ni el destino).
    pub opciones: Vec<String>,
    /// Borrar el origen al terminar (el resultado ocupa su sitio, en .mp4).
    pub reemplazar: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TipoConversion {
    /// La versión de 30 fps de un vídeo de la biblioteca (se sustituye si ya existía).
    Version30,
    /// La del diálogo «Convertir».
    Personalizada,
}

impl TipoConversion {
    fn clave(self) -> &'static str {
        match self {
            TipoConversion::Version30 => "30fps",
            TipoConversion::Personalizada => "personalizada",
        }
    }

    fn de_clave(t: &str) -> TipoConversion {
        if t == "30fps" { TipoConversion::Version30 } else { TipoConversion::Personalizada }
    }
}

/// Cómo importar: se decide al encolar (el gestor no tiene pantalla ni detecta el hardware).
#[derive(Clone, Debug, PartialEq)]
pub struct AjustesImportar {
    pub biblioteca: PathBuf,
    pub sin_audio: bool,
    pub optimizar: Option<(i32, i32)>,
    pub version_30: Option<i32>,
}

impl Trabajo {
    pub fn terminado(&self) -> bool {
        TERMINADOS.contains(&self.estado.as_str())
    }

    /// Fichero donde se va bajando (se reanuda desde él si se pausa).
    pub fn temporal(&self) -> PathBuf {
        carpeta().join(format!("descarga-{}.part", self.id))
    }

    /// Fichero donde va escribiendo una conversión (se renombra a «ruta» al acabar).
    pub fn temporal_conversion(&self) -> Option<PathBuf> {
        self.conversion.as_ref()?;
        Some(self.ruta.as_ref()?.with_extension("convirtiendo.mp4"))
    }

    /// Ficheros nuestros que sobran si el trabajo se cancela o se quita sin terminar.
    fn borrar_ficheros(&self) {
        if self.url.is_some() {
            let _ = std::fs::remove_file(self.temporal());
        }
        if let Some(t) = self.temporal_conversion() {
            let _ = std::fs::remove_file(t);
        }
        if self.mover && self.estado != LISTA {
            if let Some(f) = &self.fichero {
                let _ = std::fs::remove_file(f);
            }
        }
    }
}

/// Carpeta de las descargas a medias.
pub fn carpeta() -> PathBuf {
    config::carpeta_cache().join("descargas")
}

pub fn ahora() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn abrir() -> Result<Connection, String> {
    let dir = config::carpeta_datos();
    let _ = std::fs::create_dir_all(&dir);
    abrir_en(&dir.join("descargas.db")).map_err(|e| format!("cola de descargas: {e}"))
}

fn abrir_en(ruta: &Path) -> rusqlite::Result<Connection> {
    let con = Connection::open(ruta)?;
    con.busy_timeout(Duration::from_secs(3))?;
    // WAL: la app lee mientras el gestor escribe, sin esperas.
    con.pragma_update(None, "journal_mode", "WAL")?;
    con.execute_batch(
        "CREATE TABLE IF NOT EXISTS trabajos (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            clave      TEXT NOT NULL,
            nombre     TEXT NOT NULL,
            titulo     TEXT,
            url        TEXT,
            referer    TEXT,
            fichero    TEXT,
            mover      INTEGER NOT NULL,
            usar       INTEGER NOT NULL DEFAULT 0,
            orden      INTEGER NOT NULL,
            estado     TEXT NOT NULL,
            peticion   TEXT,
            hecho      INTEGER NOT NULL DEFAULT 0,
            total      INTEGER,
            mensaje    TEXT,
            ruta       TEXT,
            avisos     TEXT,
            avisada    INTEGER NOT NULL DEFAULT 0,
            biblioteca TEXT NOT NULL,
            sin_audio  INTEGER NOT NULL,
            opt_ancho  INTEGER,
            opt_alto   INTEGER,
            version_30 INTEGER,
            creada     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS trabajos_clave ON trabajos (clave);",
    )?;
    // Columnas añadidas después (las bases de antes no las tienen).
    let mut c = con.prepare("SELECT name FROM pragma_table_info('trabajos')")?;
    let columnas: Vec<String> = c.query_map([], |f| f.get(0))?.collect::<rusqlite::Result<_>>()?;
    drop(c);
    for (columna, tipo) in [("conversion", "TEXT"), ("opciones", "TEXT"), ("reemplazar", "INTEGER NOT NULL DEFAULT 0")] {
        if !columnas.iter().any(|c| c == columna) {
            con.execute_batch(&format!("ALTER TABLE trabajos ADD COLUMN {columna} {tipo}"))?;
        }
    }
    Ok(con)
}

const COLUMNAS: &str = "id, clave, nombre, titulo, url, referer, fichero, mover, usar, estado, peticion, hecho,
    total, mensaje, ruta, avisos, avisada, biblioteca, sin_audio, opt_ancho, opt_alto, version_30, conversion,
    opciones, reemplazar";

fn de_fila(f: &Row) -> rusqlite::Result<Trabajo> {
    let ancho: Option<i32> = f.get(19)?;
    let alto: Option<i32> = f.get(20)?;
    let avisos: Option<String> = f.get(15)?;
    let conversion = f.get::<_, Option<String>>(22)?.map(|tipo| -> rusqlite::Result<Conversion> {
        let opciones: Option<String> = f.get(23)?;
        Ok(Conversion {
            tipo: TipoConversion::de_clave(&tipo),
            opciones: opciones.and_then(|o| serde_json::from_str(&o).ok()).unwrap_or_default(),
            reemplazar: f.get(24)?,
        })
    });
    Ok(Trabajo {
        id: f.get(0)?,
        clave: f.get(1)?,
        nombre: f.get(2)?,
        titulo: f.get(3)?,
        url: f.get(4)?,
        referer: f.get(5)?,
        fichero: f.get::<_, Option<String>>(6)?.map(PathBuf::from),
        mover: f.get(7)?,
        usar: f.get(8)?,
        estado: f.get(9)?,
        peticion: f.get(10)?,
        hecho: f.get::<_, i64>(11)? as u64,
        total: f.get::<_, Option<i64>>(12)?.map(|t| t as u64),
        mensaje: f.get(13)?,
        ruta: f.get::<_, Option<String>>(14)?.map(PathBuf::from),
        avisos: avisos.map(|a| a.lines().map(str::to_string).collect()).unwrap_or_default(),
        avisada: f.get(16)?,
        ajustes: AjustesImportar {
            biblioteca: PathBuf::from(f.get::<_, String>(17)?),
            sin_audio: f.get(18)?,
            optimizar: ancho.zip(alto),
            version_30: f.get(21)?,
        },
        conversion: conversion.transpose()?,
    })
}

fn texto(e: rusqlite::Error) -> String {
    format!("cola de descargas: {e}")
}

/// Toda la cola, en orden.
pub fn listar(con: &Connection) -> Result<Vec<Trabajo>, String> {
    let mut c = con.prepare(&format!("SELECT {COLUMNAS} FROM trabajos ORDER BY orden")).map_err(texto)?;
    c.query_map([], de_fila).and_then(|f| f.collect()).map_err(texto)
}

pub fn leer(con: &Connection, id: i64) -> Result<Option<Trabajo>, String> {
    con.query_row(&format!("SELECT {COLUMNAS} FROM trabajos WHERE id = ?1"), [id], de_fila)
        .optional()
        .map_err(texto)
}

/// ¿Hay algo que el gestor tenga que hacer?
pub fn hay_trabajo(con: &Connection) -> Result<bool, String> {
    con.query_row(
        "SELECT EXISTS (SELECT 1 FROM trabajos WHERE estado IN (?1, ?2, ?3, ?4, ?5, ?6))",
        params![EN_COLA, DESCARGANDO, POR_IMPORTAR, IMPORTANDO, POR_CONVERTIR, CONVIRTIENDO],
        |f| f.get(0),
    )
    .map_err(texto)
}

#[derive(Debug, PartialEq)]
pub enum Encolado {
    Nuevo,
    /// Ya estaba esperando (o pausado, o en marcha): no se repite.
    YaEstaba,
    /// Estaba con error o cancelado: se reintenta ese mismo.
    Reintentado,
}

pub struct Nuevo<'a> {
    pub clave: &'a str,
    pub nombre: &'a str,
    pub titulo: Option<&'a str>,
    pub url: Option<&'a str>,
    pub referer: Option<&'a str>,
    pub fichero: Option<&'a Path>,
    pub mover: bool,
    pub usar: bool,
    pub ajustes: &'a AjustesImportar,
    /// Con conversión: `fichero` es el origen y `destino`, dónde dejar el resultado.
    pub conversion: Option<&'a Conversion>,
    pub destino: Option<&'a Path>,
}

/// Al final de la cola, salvo que ya esté en ella sin terminar.
pub fn encolar(con: &mut Connection, n: &Nuevo) -> Result<Encolado, String> {
    let tx = con.transaction().map_err(texto)?;
    let previo: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, estado FROM trabajos WHERE clave = ?1 AND estado != ?2 ORDER BY id DESC LIMIT 1",
            params![n.clave, LISTA],
            |f| Ok((f.get(0)?, f.get(1)?)),
        )
        .optional()
        .map_err(texto)?;
    if let Some((id, estado)) = previo {
        if estado == ERROR || estado == CANCELADA {
            // Una conversión se reintenta con las opciones de ahora (p. ej. otra calidad).
            if let Some(c) = n.conversion {
                tx.execute(
                    "UPDATE trabajos SET opciones = ?1, reemplazar = ?2 WHERE id = ?3",
                    params![serde_json::to_string(&c.opciones).unwrap_or_default(), c.reemplazar, id],
                )
                .map_err(texto)?;
            }
            tx.commit().map_err(texto)?;
            reintentar(con, id)?;
            return Ok(Encolado::Reintentado);
        }
        drop(tx);
        return Ok(Encolado::YaEstaba);
    }
    let a = n.ajustes;
    tx.execute(
        "INSERT INTO trabajos (clave, nombre, titulo, url, referer, fichero, mover, usar, orden, estado,
            biblioteca, sin_audio, opt_ancho, opt_alto, version_30, creada, ruta, conversion, opciones, reemplazar)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, (SELECT COALESCE(MAX(orden), 0) + 1 FROM trabajos), ?9,
            ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            n.clave,
            n.nombre,
            n.titulo,
            n.url,
            n.referer,
            n.fichero.map(|f| f.to_string_lossy().into_owned()),
            n.mover,
            n.usar,
            if n.conversion.is_some() {
                POR_CONVERTIR
            } else if n.url.is_some() {
                EN_COLA
            } else {
                POR_IMPORTAR
            },
            a.biblioteca.to_string_lossy(),
            a.sin_audio,
            a.optimizar.map(|o| o.0),
            a.optimizar.map(|o| o.1),
            a.version_30,
            ahora(),
            n.destino.map(|d| d.to_string_lossy().into_owned()),
            n.conversion.map(|c| c.tipo.clave()),
            n.conversion.map(|c| serde_json::to_string(&c.opciones).unwrap_or_default()),
            n.conversion.is_some_and(|c| c.reemplazar),
        ],
    )
    .map_err(texto)?;
    tx.commit().map_err(texto)?;
    Ok(Encolado::Nuevo)
}

/// La versión de 30 fps de un vídeo de la biblioteca, al final de la cola (sustituye a la que
/// hubiera al terminar).
pub fn encolar_version_30(
    con: &mut Connection,
    original: &Path,
    calidad: i32,
    ajustes: &AjustesImportar,
) -> Result<Encolado, String> {
    let conversion = Conversion {
        tipo: TipoConversion::Version30,
        opciones: crate::importar::opciones_version_30(calidad),
        reemplazar: false,
    };
    let clave = format!("30fps:{}", original.display());
    let nombre = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    encolar(
        con,
        &Nuevo {
            clave: &clave,
            nombre: &nombre,
            titulo: None,
            url: None,
            referer: None,
            fichero: Some(original),
            mover: false,
            usar: false,
            ajustes,
            conversion: Some(&conversion),
            destino: Some(&config::ruta_version_30(original)),
        },
    )
}

/// Cambia el estado solo si sigue en uno de `desde`. Devuelve si lo ha cambiado.
pub fn pasar(con: &Connection, id: i64, desde: &[&str], a: &str) -> Result<bool, String> {
    let lista = desde.iter().map(|e| format!("'{e}'")).collect::<Vec<_>>().join(", ");
    let n = con
        .execute(
            &format!("UPDATE trabajos SET estado = ?1, peticion = NULL WHERE id = ?2 AND estado IN ({lista})"),
            params![a, id],
        )
        .map_err(texto)?;
    Ok(n > 0)
}

pub fn pausar(con: &Connection, id: i64) -> Result<(), String> {
    // Una conversión en marcha no se puede pausar (solo cancelar).
    if !pasar(con, id, &[EN_COLA, POR_CONVERTIR], PAUSADA)? {
        // En marcha: se lo pide al gestor, que para cuando lea la petición.
        con.execute(
            "UPDATE trabajos SET peticion = ?1 WHERE id = ?2 AND estado = ?3 AND url IS NOT NULL",
            params![PEDIR_PAUSA, id, DESCARGANDO],
        )
        .map_err(texto)?;
    }
    Ok(())
}

pub fn reanudar(con: &Connection, id: i64) -> Result<(), String> {
    con.execute(
        "UPDATE trabajos SET estado = CASE WHEN conversion IS NULL THEN ?1 ELSE ?2 END, peticion = NULL
         WHERE id = ?3 AND estado = ?4",
        params![EN_COLA, POR_CONVERTIR, id, PAUSADA],
    )
    .map_err(texto)?;
    Ok(())
}

pub fn cancelar(con: &Connection, id: i64) -> Result<(), String> {
    let Some(t) = leer(con, id)? else { return Ok(()) };
    if pasar(con, id, &[EN_COLA, PAUSADA, POR_IMPORTAR, POR_CONVERTIR], CANCELADA)? {
        t.borrar_ficheros();
    } else {
        con.execute(
            "UPDATE trabajos SET peticion = ?1 WHERE id = ?2 AND estado IN (?3, ?4)",
            params![PEDIR_CANCELACION, id, DESCARGANDO, CONVIRTIENDO],
        )
        .map_err(texto)?;
    }
    Ok(())
}

/// Vuelve a la cola un trabajo con error o cancelado: a importar si el fichero sigue ahí, y si
/// no, a descargar (sigue donde se quedó, si queda algo bajado). Las conversiones, a convertir
/// otra vez desde el principio.
pub fn reintentar(con: &Connection, id: i64) -> Result<(), String> {
    let Some(t) = leer(con, id)? else { return Ok(()) };
    let a = if t.conversion.is_some() {
        if !t.fichero.as_deref().is_some_and(Path::is_file) {
            return Err("el vídeo original ya no está".into());
        }
        con.execute("UPDATE trabajos SET hecho = 0, total = NULL WHERE id = ?1", [id]).map_err(texto)?;
        POR_CONVERTIR
    } else if t.fichero.as_deref().is_some_and(Path::is_file) {
        POR_IMPORTAR
    } else if t.url.is_some() {
        con.execute("UPDATE trabajos SET fichero = NULL WHERE id = ?1", [id]).map_err(texto)?;
        EN_COLA
    } else {
        return Err("el fichero ya no está".into());
    };
    con.execute(
        "UPDATE trabajos SET mensaje = NULL, avisada = 0 WHERE id = ?1 AND estado IN (?2, ?3)",
        params![id, ERROR, CANCELADA],
    )
    .map_err(texto)?;
    pasar(con, id, &[ERROR, CANCELADA], a).map(|_| ())
}

/// Fuera de la lista (solo los terminados).
pub fn quitar(con: &Connection, id: i64) -> Result<(), String> {
    let Some(t) = leer(con, id)? else { return Ok(()) };
    if !t.terminado() {
        return Ok(());
    }
    con.execute("DELETE FROM trabajos WHERE id = ?1", [id]).map_err(texto)?;
    t.borrar_ficheros();
    Ok(())
}

pub fn quitar_terminados(con: &Connection) -> Result<(), String> {
    for t in listar(con)?.into_iter().filter(Trabajo::terminado) {
        quitar(con, t.id)?;
    }
    Ok(())
}

/// Sube (−1) o baja (+1) un puesto en la cola.
pub fn mover(con: &mut Connection, id: i64, sentido: i32) -> Result<(), String> {
    let tx = con.transaction().map_err(texto)?;
    let orden: i64 = tx.query_row("SELECT orden FROM trabajos WHERE id = ?1", [id], |f| f.get(0)).map_err(texto)?;
    let consulta = if sentido < 0 {
        "SELECT id, orden FROM trabajos WHERE orden < ?1 ORDER BY orden DESC LIMIT 1"
    } else {
        "SELECT id, orden FROM trabajos WHERE orden > ?1 ORDER BY orden LIMIT 1"
    };
    let vecino: Option<(i64, i64)> =
        tx.query_row(consulta, [orden], |f| Ok((f.get(0)?, f.get(1)?))).optional().map_err(texto)?;
    if let Some((otro, su_orden)) = vecino {
        tx.execute("UPDATE trabajos SET orden = ?1 WHERE id = ?2", params![su_orden, id]).map_err(texto)?;
        tx.execute("UPDATE trabajos SET orden = ?1 WHERE id = ?2", params![orden, otro]).map_err(texto)?;
    }
    tx.commit().map_err(texto)
}

/// La app ya ha enseñado el resultado.
pub fn marcar_avisada(con: &Connection, id: i64) -> Result<(), String> {
    con.execute("UPDATE trabajos SET avisada = ?1 WHERE id = ?2", params![ahora(), id]).map_err(texto)?;
    Ok(())
}

/// Las terminadas bien se quitan solas un rato después de avisar.
pub fn olvidar_listas(con: &Connection, segundos: i64) -> Result<(), String> {
    con.execute(
        "DELETE FROM trabajos WHERE estado = ?1 AND avisada > 0 AND avisada < ?2",
        params![LISTA, ahora() - segundos],
    )
    .map_err(texto)?;
    Ok(())
}

// ── Lo que usa el gestor ──

/// Recuperación al arrancar el gestor: lo que estaba en marcha cuando se cayó vuelve a la cola
/// (o se queda como pidió la app).
pub fn recuperar(con: &Connection) -> Result<(), String> {
    con.execute_batch(&format!(
        "UPDATE trabajos SET estado = '{PAUSADA}', peticion = NULL
            WHERE estado = '{DESCARGANDO}' AND peticion = '{PEDIR_PAUSA}';
         UPDATE trabajos SET estado = '{CANCELADA}', peticion = NULL
            WHERE estado = '{DESCARGANDO}' AND peticion = '{PEDIR_CANCELACION}';
         UPDATE trabajos SET estado = '{EN_COLA}', peticion = NULL WHERE estado = '{DESCARGANDO}';
         UPDATE trabajos SET estado = '{POR_IMPORTAR}' WHERE estado = '{IMPORTANDO}';
         UPDATE trabajos SET estado = '{CANCELADA}', peticion = NULL
            WHERE estado = '{CONVIRTIENDO}' AND peticion = '{PEDIR_CANCELACION}';
         UPDATE trabajos SET estado = '{POR_CONVERTIR}', peticion = NULL, hecho = 0, total = NULL
            WHERE estado = '{CONVIRTIENDO}';"
    ))
    .map_err(texto)?;
    for t in listar(con)?.into_iter().filter(|t| t.estado == CANCELADA) {
        t.borrar_ficheros();
    }
    Ok(())
}

/// El primero de la cola en `desde`, ya pasado a `a`.
pub fn reclamar(con: &Connection, desde: &str, a: &str) -> Result<Option<Trabajo>, String> {
    loop {
        let id: Option<i64> = con
            .query_row("SELECT id FROM trabajos WHERE estado = ?1 ORDER BY orden LIMIT 1", [desde], |f| f.get(0))
            .optional()
            .map_err(texto)?;
        let Some(id) = id else { return Ok(None) };
        // Si la app lo ha cambiado entretanto, a por el siguiente.
        if pasar(con, id, &[desde], a)? {
            return leer(con, id);
        }
    }
}

/// Progreso de una descarga; devuelve lo que pide la app (pausar o cancelar), si pide algo.
pub fn progreso(con: &Connection, id: i64, hecho: u64, total: Option<u64>) -> Result<Option<String>, String> {
    con.execute(
        "UPDATE trabajos SET hecho = ?1, total = ?2 WHERE id = ?3",
        params![hecho as i64, total.map(|t| t as i64), id],
    )
    .map_err(texto)?;
    con.query_row("SELECT peticion FROM trabajos WHERE id = ?1", [id], |f| f.get(0)).map_err(texto)
}

pub fn descargada(con: &Connection, id: i64, fichero: &Path) -> Result<(), String> {
    con.execute(
        "UPDATE trabajos SET estado = ?1, peticion = NULL, fichero = ?2 WHERE id = ?3",
        params![POR_IMPORTAR, fichero.to_string_lossy(), id],
    )
    .map_err(texto)?;
    Ok(())
}

pub fn fallo(con: &Connection, id: i64, error: &str) -> Result<(), String> {
    con.execute(
        "UPDATE trabajos SET estado = ?1, peticion = NULL, mensaje = ?2, avisada = 0 WHERE id = ?3",
        params![ERROR, error, id],
    )
    .map_err(texto)?;
    Ok(())
}

/// Nota sobre un trabajo en marcha (con qué codificador va una conversión).
pub fn nota(con: &Connection, id: i64, nota: &str) -> Result<(), String> {
    con.execute("UPDATE trabajos SET mensaje = ?1 WHERE id = ?2", params![nota, id]).map_err(texto)?;
    Ok(())
}

pub fn importada(con: &Connection, id: i64, ruta: &Path, resumen: &str, avisos: &[String]) -> Result<(), String> {
    con.execute(
        "UPDATE trabajos SET estado = ?1, ruta = ?2, mensaje = ?3, avisos = ?4, avisada = 0, fichero = NULL
         WHERE id = ?5",
        params![LISTA, ruta.to_string_lossy(), resumen, avisos.join("\n"), id],
    )
    .map_err(texto)?;
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn con() -> (Connection, PathBuf) {
        let dir = std::env::temp_dir().join(format!("fondos-prueba-cola-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (abrir_en(&dir.join("cola.db")).unwrap(), dir)
    }

    fn ajustes() -> AjustesImportar {
        AjustesImportar { biblioteca: "/tmp/b".into(), sin_audio: true, optimizar: Some((1920, 1080)), version_30: None }
    }

    fn web<'a>(clave: &'a str, a: &'a AjustesImportar) -> Nuevo<'a> {
        Nuevo {
            clave,
            nombre: clave,
            titulo: Some(clave),
            url: Some("https://x/dl"),
            referer: Some(clave),
            fichero: None,
            mover: true,
            usar: false,
            ajustes: a,
            conversion: None,
            destino: None,
        }
    }

    #[test]
    fn repetidos_orden_y_estados() {
        let (mut con, dir) = con();
        let a = ajustes();
        assert_eq!(encolar(&mut con, &web("a", &a)).unwrap(), Encolado::Nuevo);
        assert_eq!(encolar(&mut con, &web("b", &a)).unwrap(), Encolado::Nuevo);
        // El mismo vídeo otra vez: no entra.
        assert_eq!(encolar(&mut con, &web("a", &a)).unwrap(), Encolado::YaEstaba);
        let l = listar(&con).unwrap();
        assert_eq!(l.iter().map(|t| t.clave.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(l[0].ajustes, a);

        // Reordenar.
        mover(&mut con, l[1].id, -1).unwrap();
        assert_eq!(listar(&con).unwrap()[0].clave, "b");
        mover(&mut con, l[1].id, -1).unwrap(); // ya es el primero: nada
        assert_eq!(listar(&con).unwrap()[0].clave, "b");

        // Pausado no se reclama; el siguiente sí.
        pausar(&con, l[1].id).unwrap();
        let t = reclamar(&con, EN_COLA, DESCARGANDO).unwrap().unwrap();
        assert_eq!(t.clave, "a");
        assert!(reclamar(&con, EN_COLA, DESCARGANDO).unwrap().is_none());

        // En marcha, pausar y cancelar son peticiones al gestor.
        cancelar(&con, t.id).unwrap();
        assert_eq!(progreso(&con, t.id, 10, Some(20)).unwrap().as_deref(), Some(PEDIR_CANCELACION));
        // Si el gestor se cae, al volver respeta la petición.
        recuperar(&con).unwrap();
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, CANCELADA);

        // Cancelado y encolado otra vez: se reintenta el mismo, sin fila nueva.
        assert_eq!(encolar(&mut con, &web("a", &a)).unwrap(), Encolado::Reintentado);
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, EN_COLA);
        assert_eq!(listar(&con).unwrap().len(), 2);

        // Una vez en la biblioteca, se puede volver a bajar.
        importada(&con, t.id, Path::new("/tmp/b/a.mp4"), "1920×1080", &["x".into()]).unwrap();
        assert_eq!(encolar(&mut con, &web("a", &a)).unwrap(), Encolado::Nuevo);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recuperar_tras_caida() {
        let (mut con, dir) = con();
        let a = ajustes();
        encolar(&mut con, &web("a", &a)).unwrap();
        encolar(&mut con, &web("b", &a)).unwrap();
        let t = reclamar(&con, EN_COLA, DESCARGANDO).unwrap().unwrap();
        pausar(&con, t.id).unwrap();
        let t2 = reclamar(&con, EN_COLA, DESCARGANDO).unwrap().unwrap();
        recuperar(&con).unwrap();
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, PAUSADA);
        assert_eq!(leer(&con, t2.id).unwrap().unwrap().estado, EN_COLA);
        assert!(hay_trabajo(&con).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn conversiones() {
        let (mut con, dir) = con();
        let a = ajustes();
        let original = dir.join("rápido.mp4");
        std::fs::write(&original, b"x").unwrap();
        encolar(&mut con, &web("a", &a)).unwrap();
        assert_eq!(encolar_version_30(&mut con, &original, 28, &a).unwrap(), Encolado::Nuevo);
        assert_eq!(encolar_version_30(&mut con, &original, 28, &a).unwrap(), Encolado::YaEstaba);
        let t = listar(&con).unwrap().pop().unwrap();
        assert_eq!(t.estado, POR_CONVERTIR);
        assert_eq!(t.ruta.as_deref(), Some(dir.join("30 fps").join("rápido.mp4").as_path()));
        let c = t.conversion.clone().unwrap();
        assert_eq!(c.tipo, TipoConversion::Version30);
        assert_eq!(c.opciones, ["-f", "30", "-q", "28"]);
        assert_eq!(t.temporal_conversion().unwrap(), dir.join("30 fps").join("rápido.convirtiendo.mp4"));

        // Las descargas no se llevan las conversiones, ni al revés.
        assert_eq!(reclamar(&con, EN_COLA, DESCARGANDO).unwrap().unwrap().clave, "a");
        assert!(reclamar(&con, EN_COLA, DESCARGANDO).unwrap().is_none());

        // En espera se pausa y se reanuda como conversión.
        pausar(&con, t.id).unwrap();
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, PAUSADA);
        reanudar(&con, t.id).unwrap();
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, POR_CONVERTIR);

        // En marcha: cancelar es una petición; si el gestor se cae, al volver la respeta.
        let t = reclamar(&con, POR_CONVERTIR, CONVIRTIENDO).unwrap().unwrap();
        pausar(&con, t.id).unwrap(); // no hace nada
        assert_eq!(progreso(&con, t.id, 1, Some(2)).unwrap(), None);
        cancelar(&con, t.id).unwrap();
        assert_eq!(progreso(&con, t.id, 1, Some(2)).unwrap().as_deref(), Some(PEDIR_CANCELACION));
        recuperar(&con).unwrap();
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, CANCELADA);

        // Otra vez a la cola, con otra calidad: el mismo trabajo, con las opciones nuevas.
        assert_eq!(encolar_version_30(&mut con, &original, 24, &a).unwrap(), Encolado::Reintentado);
        let t = leer(&con, t.id).unwrap().unwrap();
        assert_eq!((t.estado.as_str(), t.conversion.unwrap().opciones[3].as_str()), (POR_CONVERTIR, "24"));

        // Si el gestor se cae a mitad de una conversión, vuelve a empezar.
        reclamar(&con, POR_CONVERTIR, CONVIRTIENDO).unwrap().unwrap();
        recuperar(&con).unwrap();
        assert_eq!(leer(&con, t.id).unwrap().unwrap().estado, POR_CONVERTIR);
        assert!(hay_trabajo(&con).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Una base de antes de las conversiones gana sus columnas al abrirla.
    #[test]
    fn base_antigua() {
        let dir = std::env::temp_dir().join(format!("fondos-prueba-cola-antigua-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ruta = dir.join("cola.db");
        Connection::open(&ruta)
            .unwrap()
            .execute_batch(
                "CREATE TABLE trabajos (id INTEGER PRIMARY KEY AUTOINCREMENT, clave TEXT NOT NULL,
                    nombre TEXT NOT NULL, titulo TEXT, url TEXT, referer TEXT, fichero TEXT,
                    mover INTEGER NOT NULL, usar INTEGER NOT NULL DEFAULT 0, orden INTEGER NOT NULL,
                    estado TEXT NOT NULL, peticion TEXT, hecho INTEGER NOT NULL DEFAULT 0, total INTEGER,
                    mensaje TEXT, ruta TEXT, avisos TEXT, avisada INTEGER NOT NULL DEFAULT 0,
                    biblioteca TEXT NOT NULL, sin_audio INTEGER NOT NULL, opt_ancho INTEGER,
                    opt_alto INTEGER, version_30 INTEGER, creada INTEGER NOT NULL);
                 INSERT INTO trabajos (clave, nombre, mover, orden, estado, biblioteca, sin_audio, creada)
                    VALUES ('a', 'a', 1, 1, 'en_cola', '/tmp/b', 1, 0);",
            )
            .unwrap();
        let mut con = abrir_en(&ruta).unwrap();
        assert_eq!(listar(&con).unwrap()[0].conversion, None);
        encolar_version_30(&mut con, Path::new("/tmp/b/x.mp4"), 28, &ajustes()).unwrap();
        assert!(listar(&con).unwrap()[1].conversion.is_some());
        drop(con);
        abrir_en(&ruta).unwrap(); // y abrirla otra vez no falla
        let _ = std::fs::remove_dir_all(&dir);
    }
}
