//! Cola de descargas e importaciones (SQLite, en ~/.local/share/fondos-animados/descargas.db).
//! La comparten la app, que encola y pide cosas (pausar, cancelar, reordenar…), y el gestor de
//! descargas (gestor.rs), un proceso aparte que hace el trabajo: si uno de los dos se cae, la
//! cola sigue ahí. Los cambios de estado se hacen con UPDATE … WHERE estado = …, para que la app
//! y el gestor no se pisen.
//!
//! Estados: en_cola → descargando → por_importar → importando → lista, más pausada, error y
//! cancelada. Los ficheros locales y los que baja el navegador integrado entran ya por_importar.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::config;

pub const EN_COLA: &str = "en_cola";
pub const DESCARGANDO: &str = "descargando";
pub const PAUSADA: &str = "pausada";
pub const POR_IMPORTAR: &str = "por_importar";
pub const IMPORTANDO: &str = "importando";
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

    /// Ficheros nuestros que sobran si el trabajo se cancela o se quita sin terminar.
    fn borrar_ficheros(&self) {
        if self.url.is_some() {
            let _ = std::fs::remove_file(self.temporal());
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
    Ok(con)
}

const COLUMNAS: &str = "id, clave, nombre, titulo, url, referer, fichero, mover, usar, estado, peticion, hecho,
    total, mensaje, ruta, avisos, avisada, biblioteca, sin_audio, opt_ancho, opt_alto, version_30";

fn de_fila(f: &Row) -> rusqlite::Result<Trabajo> {
    let ancho: Option<i32> = f.get(19)?;
    let alto: Option<i32> = f.get(20)?;
    let avisos: Option<String> = f.get(15)?;
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
        "SELECT EXISTS (SELECT 1 FROM trabajos WHERE estado IN (?1, ?2, ?3, ?4))",
        params![EN_COLA, DESCARGANDO, POR_IMPORTAR, IMPORTANDO],
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
        drop(tx);
        if estado == ERROR || estado == CANCELADA {
            reintentar(con, id)?;
            return Ok(Encolado::Reintentado);
        }
        return Ok(Encolado::YaEstaba);
    }
    let a = n.ajustes;
    tx.execute(
        "INSERT INTO trabajos (clave, nombre, titulo, url, referer, fichero, mover, usar, orden, estado,
            biblioteca, sin_audio, opt_ancho, opt_alto, version_30, creada)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, (SELECT COALESCE(MAX(orden), 0) + 1 FROM trabajos), ?9,
            ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            n.clave,
            n.nombre,
            n.titulo,
            n.url,
            n.referer,
            n.fichero.map(|f| f.to_string_lossy().into_owned()),
            n.mover,
            n.usar,
            if n.url.is_some() { EN_COLA } else { POR_IMPORTAR },
            a.biblioteca.to_string_lossy(),
            a.sin_audio,
            a.optimizar.map(|o| o.0),
            a.optimizar.map(|o| o.1),
            a.version_30,
            ahora(),
        ],
    )
    .map_err(texto)?;
    tx.commit().map_err(texto)?;
    Ok(Encolado::Nuevo)
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
    if !pasar(con, id, &[EN_COLA], PAUSADA)? {
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
    pasar(con, id, &[PAUSADA], EN_COLA).map(|_| ())
}

pub fn cancelar(con: &Connection, id: i64) -> Result<(), String> {
    let Some(t) = leer(con, id)? else { return Ok(()) };
    if pasar(con, id, &[EN_COLA, PAUSADA, POR_IMPORTAR], CANCELADA)? {
        t.borrar_ficheros();
    } else {
        con.execute(
            "UPDATE trabajos SET peticion = ?1 WHERE id = ?2 AND estado = ?3",
            params![PEDIR_CANCELACION, id, DESCARGANDO],
        )
        .map_err(texto)?;
    }
    Ok(())
}

/// Vuelve a la cola un trabajo con error o cancelado: a importar si el fichero sigue ahí, y si
/// no, a descargar (sigue donde se quedó, si queda algo bajado).
pub fn reintentar(con: &Connection, id: i64) -> Result<(), String> {
    let Some(t) = leer(con, id)? else { return Ok(()) };
    let a = if t.fichero.as_deref().is_some_and(Path::is_file) {
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
         UPDATE trabajos SET estado = '{POR_IMPORTAR}' WHERE estado = '{IMPORTANDO}';"
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
}
