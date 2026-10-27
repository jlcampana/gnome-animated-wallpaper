//! Exportar la lista de fondos de la biblioteca a un JSON (qué hay, de qué web salió cada uno y
//! cómo bajarlo) e importarla en otro equipo, que se baja los que le falten. Para compartir
//! fondos entre equipos sin copiar los vídeos.
//!
//! El origen de cada vídeo lo apunta el gestor al importarlo (bd.rs, tabla «origenes»). Los que
//! se bajaron antes de que se apuntara (o los ficheros que añade el usuario) se buscan por su
//! nombre en las webs, en segundo plano al abrir la app y al exportar: el fichero se llama como
//! el título del resultado (config::nombre_limpio), así que si un resultado da ese mismo
//! nombre, es él. Cada vídeo se busca una sola vez: lo que no se encuentra queda apuntado en
//! la tabla «sin_origen».

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};

use crate::bd::{self, Origen};
use crate::contexto::Contexto;
use crate::descargas::Descargas;
use crate::fuentes::Fuente;
use crate::{biblioteca::Biblioteca, config};

const FORMATO: &str = "fondos-animados/lista";
const VERSION: i64 = 1;

/// Un fondo de la lista.
#[derive(Clone, Debug, PartialEq)]
pub struct Entrada {
    pub titulo: String,
    /// Nombre del fichero sin extensión: con él se ve si ya está en la biblioteca.
    pub nombre: String,
    pub pagina: Option<String>,
    pub descarga: Option<String>,
    pub resolucion: Option<(u32, u32)>,
    pub fps: Option<f64>,
}

fn nombre(ruta: &Path) -> String {
    ruta.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// «https://moewalls.com/anime/x/» → «moewalls.com».
fn web_de(pagina: &str) -> &str {
    let sin = pagina.split_once("://").map_or(pagina, |(_, r)| r);
    sin.split('/').next().unwrap_or(sin)
}

/// Qué buscar para dar con `nombre`: todo; sin las palabras de una letra (a los buscadores de
/// las webs les sientan mal: «sunset rx 7» no da nada, «sunset rx» sí); y solo las dos
/// primeras de esas.
fn consultas(nombre: &str) -> Vec<String> {
    let palabras: Vec<&str> = nombre.split('-').filter(|p| !p.is_empty()).collect();
    let largas: Vec<&str> = palabras.iter().copied().filter(|p| p.chars().count() > 1).collect();
    let mut r: Vec<String> = vec![];
    for c in [palabras.join(" "), largas.join(" "), largas.iter().take(2).copied().collect::<Vec<_>>().join(" ")] {
        if !c.is_empty() && !r.contains(&c) {
            r.push(c);
        }
    }
    r
}

/// Busca en las webs nativas un resultado que dé el mismo nombre de fichero; con él, la
/// descarga (la 4K de motionbgs si el vídeo es 4K). Bloqueante.
pub fn buscar_origen(nombre: &str, ancho: Option<u32>) -> Option<Origen> {
    consultas(nombre).iter().find_map(|c| buscar_con(c, nombre, ancho))
}

fn buscar_con(consulta: &str, nombre: &str, ancho: Option<u32>) -> Option<Origen> {
    for fuente in Fuente::TODAS.into_iter().filter(|f| f.nativa()) {
        let resultados = match fuente.buscar(&consulta, 0, None, 1) {
            Ok(r) => r,
            Err(e) => {
                crate::depurar!("buscar «{consulta}» en {}: {e}", fuente.nombre());
                continue;
            }
        };
        let Some(r) = resultados.into_iter().find(|r| config::nombre_limpio(&r.titulo) == nombre) else { continue };
        let detalle = match fuente.detalle(&r) {
            Ok(d) => d,
            Err(e) => {
                crate::depurar!("detalle de {}: {e}", r.pagina);
                continue;
            }
        };
        let en_4k = ancho.is_some_and(|a| a >= 3840);
        let descarga = detalle
            .descargas
            .iter()
            .find(|d| d.etiqueta.starts_with("4K") == en_4k)
            .or(detalle.descargas.first())
            .map(|d| d.url.clone());
        crate::depurar!("origen de «{nombre}»: {}", r.pagina);
        return Some(Origen { pagina: r.pagina, descarga, titulo: Some(r.titulo) });
    }
    None
}

/// Los de `videos` cuyo origen no se sabe ni se ha buscado ya.
pub fn por_recomponer(videos: &[PathBuf]) -> Vec<PathBuf> {
    let origenes = bd::origenes();
    let buscados = bd::sin_origen();
    videos.iter().filter(|v| !origenes.contains_key(*v) && !buscados.contains(*v)).cloned().collect()
}

/// Busca en las webs el origen de los `videos` que no lo tengan y lo guarda. Devuelve cuántos
/// ha encontrado. Bloqueante.
pub fn recomponer(videos: &[PathBuf]) -> usize {
    let pendientes = por_recomponer(videos);
    if pendientes.is_empty() {
        return 0;
    }
    let infos = bd::sincronizar(&pendientes);
    let mut encontrados = 0;
    for ruta in &pendientes {
        let r = match buscar_origen(&nombre(ruta), infos.get(ruta).map(|i| i.ancho)) {
            Some(o) => {
                encontrados += 1;
                bd::guardar_origen(ruta, &o)
            }
            None => bd::marcar_sin_origen(ruta),
        };
        if let Err(e) = r {
            crate::depurar!("origen de {}: {e}", ruta.display());
        }
    }
    crate::depurar!("orígenes recompuestos: {encontrados} de {}", pendientes.len());
    encontrados
}

/// Al abrir la app: recompone en segundo plano los orígenes que falten (p. ej. los de los
/// vídeos bajados con versiones que no los guardaban).
pub fn recomponer_al_arrancar(biblioteca: &Rc<Biblioteca>) {
    let biblioteca = biblioteca.clone();
    // Un poco después, para no competir con la carga de la biblioteca.
    glib::timeout_add_seconds_local_once(10, move || {
        let videos = biblioteca.principales();
        glib::spawn_future_local(async move {
            let _ = gio::spawn_blocking(move || recomponer(&videos)).await;
        });
    });
}

/// La lista de `videos` (el principal de cada fondo), buscando antes el origen de los que no
/// lo tengan apuntado. Bloqueante.
pub fn reunir(videos: &[PathBuf]) -> Vec<Entrada> {
    recomponer(videos);
    let infos = bd::sincronizar(videos);
    let origenes = bd::origenes();
    videos
        .iter()
        .map(|ruta| {
            let info = infos.get(ruta);
            let nombre = nombre(ruta);
            let origen = origenes.get(ruta).cloned();
            Entrada {
                titulo: origen.as_ref().and_then(|o| o.titulo.clone()).unwrap_or_else(|| nombre.clone()),
                nombre,
                pagina: origen.as_ref().map(|o| o.pagina.clone()),
                descarga: origen.and_then(|o| o.descarga),
                resolucion: info.map(|i| (i.ancho, i.alto)),
                fps: info.map(|i| i.fps),
            }
        })
        .collect()
}

pub fn a_json(entradas: &[Entrada]) -> String {
    let fondos: Vec<Value> = entradas
        .iter()
        .map(|e| {
            json!({
                "titulo": e.titulo,
                "nombre": e.nombre,
                "web": e.pagina.as_deref().map(web_de),
                "pagina": e.pagina,
                "descarga": e.descarga,
                "resolucion": e.resolucion.map(|(w, h)| format!("{w}x{h}")),
                "fps": e.fps.map(|f| (f * 100.0).round() / 100.0),
            })
        })
        .collect();
    let doc = json!({
        "formato": FORMATO,
        "version": VERSION,
        "app": env!("CARGO_PKG_VERSION"),
        "equipo": glib::host_name().as_str(),
        "fecha": glib::DateTime::now_local().ok().and_then(|d| d.format_iso8601().ok()).map(|f| f.to_string()),
        "fondos": fondos,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

pub fn de_json(texto: &str) -> Result<Vec<Entrada>, String> {
    let doc: Value = serde_json::from_str(texto).map_err(|e| format!("no es un JSON válido: {e}"))?;
    if doc["formato"] != FORMATO {
        return Err("no es una lista de fondos exportada con Fondos animados".into());
    }
    if doc["version"].as_i64().is_none_or(|v| v > VERSION) {
        return Err("la lista es de una versión más nueva de la app: actualízala".into());
    }
    let texto = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let fondos = doc["fondos"].as_array().ok_or("la lista no tiene fondos")?;
    Ok(fondos
        .iter()
        .filter_map(|f| {
            let nombre = texto(&f["nombre"])?;
            Some(Entrada {
                titulo: texto(&f["titulo"]).unwrap_or_else(|| nombre.clone()),
                nombre,
                pagina: texto(&f["pagina"]),
                descarga: texto(&f["descarga"]),
                resolucion: f["resolucion"].as_str().and_then(|r| {
                    let (w, h) = r.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                }),
                fps: f["fps"].as_f64(),
            })
        })
        .collect())
}

/// Qué hacer con cada fondo de una lista importada.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub bajar: Vec<Entrada>,
    pub ya_estan: Vec<Entrada>,
    /// Solo se pueden bajar con el navegador integrado (vsthemes).
    pub navegador: Vec<Entrada>,
    pub sin_origen: Vec<Entrada>,
}

/// Reparte la lista: ya está si hay un fondo con ese nombre o bajado de esa misma página.
pub fn planificar(entradas: Vec<Entrada>, nombres: &HashSet<String>, paginas: &HashSet<String>) -> Plan {
    let mut plan = Plan::default();
    let mut vistos = HashSet::new();
    for e in entradas {
        if !vistos.insert(e.pagina.clone().unwrap_or_else(|| e.nombre.clone())) {
            continue;
        }
        let lista = if nombres.contains(&e.nombre) || e.pagina.as_ref().is_some_and(|p| paginas.contains(p)) {
            &mut plan.ya_estan
        } else if e.descarga.is_some() {
            &mut plan.bajar
        } else if e.pagina.is_some() {
            &mut plan.navegador
        } else {
            &mut plan.sin_origen
        };
        lista.push(e);
    }
    plan
}

// ── En la ventana ──

fn filtro_json() -> gio::ListStore {
    let filtro = gtk::FileFilter::new();
    filtro.set_name(Some("Listas de fondos (JSON)"));
    filtro.add_suffix("json");
    let filtros = gio::ListStore::new::<gtk::FileFilter>();
    filtros.append(&filtro);
    filtros
}

/// Pide dónde guardar la lista y la guarda (buscando antes el origen de los que no lo tengan).
pub fn exportar(ctx: &Rc<Contexto>, biblioteca: &Rc<Biblioteca>) {
    let videos = biblioteca.principales();
    if videos.is_empty() {
        ctx.aviso("La biblioteca está vacía: no hay nada que exportar");
        return;
    }
    let dialogo = gtk::FileDialog::builder()
        .title("Exportar la lista de fondos")
        .initial_name(format!("fondos-{}.json", glib::host_name()))
        .filters(&filtro_json())
        .build();
    let ctx = ctx.clone();
    dialogo.save(Some(&ctx.ventana.clone()), gio::Cancellable::NONE, move |r| {
        let Some(destino) = r.ok().and_then(|f| f.path()) else { return };
        let faltan = por_recomponer(&videos).len();
        if faltan > 0 {
            ctx.aviso(&format!("Buscando en las webs de dónde salieron {faltan} fondo(s)…"));
        }
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || {
                let entradas = reunir(&videos);
                std::fs::write(&destino, a_json(&entradas)).map(|_| entradas).map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("ha fallado la exportación".into()));
            match r {
                Ok(entradas) => {
                    let sin = entradas.iter().filter(|e| e.pagina.is_none()).count();
                    let mut texto = format!("Lista de {} fondo(s) exportada", entradas.len());
                    if sin > 0 {
                        texto.push_str(&format!("; de {sin} no se sabe de dónde salieron"));
                    }
                    ctx.aviso(&texto);
                }
                Err(e) => ctx.aviso(&format!("No se pudo exportar: {e}")),
            }
        });
    });
}

/// Pide una lista exportada y, tras confirmar, pone a descargar los fondos que falten.
pub fn importar(ctx: &Rc<Contexto>, biblioteca: &Rc<Biblioteca>, descargas: &Rc<Descargas>) {
    let dialogo = gtk::FileDialog::builder().title("Importar una lista de fondos").filters(&filtro_json()).build();
    let ctx = ctx.clone();
    let biblioteca = biblioteca.clone();
    let descargas = descargas.clone();
    dialogo.open(Some(&ctx.ventana.clone()), gio::Cancellable::NONE, move |r| {
        let Some(fichero) = r.ok().and_then(|f| f.path()) else { return };
        let videos = biblioteca.principales();
        glib::spawn_future_local(async move {
            let plan = gio::spawn_blocking(move || -> Result<Plan, String> {
                let texto = std::fs::read_to_string(&fichero).map_err(|e| e.to_string())?;
                let entradas = de_json(&texto)?;
                let nombres: HashSet<String> = videos.iter().map(|v| nombre(v)).collect();
                let presentes: HashSet<&PathBuf> = videos.iter().collect();
                let paginas = bd::origenes()
                    .into_iter()
                    .filter(|(ruta, _)| presentes.contains(ruta))
                    .map(|(_, o)| o.pagina)
                    .collect();
                Ok(planificar(entradas, &nombres, &paginas))
            })
            .await
            .unwrap_or_else(|_| Err("ha fallado la importación".into()));
            match plan {
                Ok(plan) => confirmar(&ctx, &descargas, plan).await,
                Err(e) => ctx.aviso(&format!("No se pudo leer la lista: {e}")),
            }
        });
    });
}

async fn confirmar(ctx: &Rc<Contexto>, descargas: &Rc<Descargas>, plan: Plan) {
    let lista = |titulo: &str, entradas: &[Entrada], con_pagina: bool| -> Option<String> {
        if entradas.is_empty() {
            return None;
        }
        let lineas: Vec<String> = entradas
            .iter()
            .map(|e| match (&e.pagina, con_pagina) {
                (Some(p), true) => format!("• {} — {p}", e.titulo),
                _ => format!("• {}", e.titulo),
            })
            .collect();
        Some(format!("{titulo} ({}):\n{}", entradas.len(), lineas.join("\n")))
    };
    let detalle: Vec<String> = [
        lista("Se descargan", &plan.bajar, false),
        lista("Solo se pueden bajar con el navegador integrado", &plan.navegador, true),
        lista("No se sabe de dónde salieron", &plan.sin_origen, false),
        lista("Ya están en la biblioteca", &plan.ya_estan, false),
    ]
    .into_iter()
    .flatten()
    .collect();

    let (titulo, cuerpo) = if plan.bajar.is_empty() {
        ("No hay fondos que descargar".to_string(), "Todos los de la lista ya están o no se pueden bajar solos.".to_string())
    } else {
        (
            format!("¿Descargar {} fondo(s)?", plan.bajar.len()),
            "Van a la cola de descargas: no hace falta dejar la app abierta.".to_string(),
        )
    };
    let dialogo = adw::AlertDialog::new(Some(&titulo), Some(&cuerpo));
    let texto = gtk::Label::builder()
        .label(detalle.join("\n\n"))
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .build();
    dialogo.set_extra_child(Some(
        &gtk::ScrolledWindow::builder()
            .child(&texto)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(320)
            .build(),
    ));
    if plan.bajar.is_empty() {
        dialogo.add_response("cerrar", "Cerrar");
        dialogo.choose_future(Some(&ctx.ventana)).await;
        return;
    }
    dialogo.add_responses(&[("cancelar", "Cancelar"), ("descargar", "Descargar")]);
    dialogo.set_response_appearance("descargar", adw::ResponseAppearance::Suggested);
    dialogo.set_default_response(Some("descargar"));
    if dialogo.choose_future(Some(&ctx.ventana)).await == "descargar" {
        let lista: Vec<(String, String, String)> = plan
            .bajar
            .into_iter()
            .filter_map(|e| Some((e.titulo, e.descarga?, e.pagina?)))
            .collect();
        descargas.descargar_lista(&lista);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn entrada(nombre: &str, pagina: Option<&str>, descarga: Option<&str>) -> Entrada {
        Entrada {
            titulo: nombre.to_uppercase(),
            nombre: nombre.into(),
            pagina: pagina.map(str::to_string),
            descarga: descarga.map(str::to_string),
            resolucion: Some((1920, 1080)),
            fps: Some(29.97),
        }
    }

    #[test]
    fn que_buscar() {
        assert_eq!(consultas("sunset-rx-7"), ["sunset rx 7", "sunset rx"]);
        assert_eq!(consultas("firefly-s-gaming-room"), ["firefly s gaming room", "firefly gaming room", "firefly gaming"]);
        assert_eq!(consultas("frieren"), ["frieren"]);
    }

    #[test]
    fn ida_y_vuelta() {
        let entradas = vec![
            entrada("sunset-rx-7", Some("https://moewalls.com/cars/sunset-rx-7/"), Some("https://go.moewalls.com/d?v=1")),
            entrada("mio", None, None),
        ];
        let texto = a_json(&entradas);
        assert!(texto.contains("\"web\": \"moewalls.com\""));
        assert_eq!(de_json(&texto).unwrap(), entradas);
        assert!(de_json("{\"fondos\": []}").is_err());
        assert!(de_json(&texto.replace("\"version\": 1", "\"version\": 99")).is_err());
    }

    #[test]
    fn plan() {
        let entradas = vec![
            entrada("ya-por-nombre", Some("https://x/1/"), Some("https://x/dl/1")),
            entrada("otro-nombre", Some("https://x/2/"), Some("https://x/dl/2")),
            entrada("nuevo", Some("https://x/3/"), Some("https://x/dl/3")),
            entrada("nuevo-repetido", Some("https://x/3/"), Some("https://x/dl/3")),
            entrada("de-vsthemes", Some("https://vsthemes.org/x/"), None),
            entrada("suelto", None, None),
        ];
        let nombres = HashSet::from(["ya-por-nombre".to_string()]);
        let paginas = HashSet::from(["https://x/2/".to_string()]);
        let p = planificar(entradas, &nombres, &paginas);
        let n = |l: &[Entrada]| l.iter().map(|e| e.nombre.clone()).collect::<Vec<_>>();
        assert_eq!(n(&p.ya_estan), ["ya-por-nombre", "otro-nombre"]);
        assert_eq!(n(&p.bajar), ["nuevo"]);
        assert_eq!(n(&p.navegador), ["de-vsthemes"]);
        assert_eq!(n(&p.sin_origen), ["suelto"]);
    }
}
