//! HTTP bloqueante (se llama siempre desde gio::spawn_blocking, nunca desde el hilo de GTK).

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use crate::config::AGENTE;

fn agente() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .user_agent(AGENTE)
        .build()
        .into()
}

pub fn texto(url: &str, referer: Option<&str>) -> Result<String, String> {
    crate::depurar!("GET {url}");
    let mut peticion = agente().get(url);
    if let Some(r) = referer {
        peticion = peticion.header("Referer", r);
    }
    let mut respuesta = peticion.call().map_err(|e| format!("{url}: {e}"))?;
    respuesta
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| format!("{url}: {e}"))
}

pub fn bytes(url: &str, referer: Option<&str>) -> Result<Vec<u8>, String> {
    crate::depurar!("GET (bytes) {url}");
    let mut peticion = agente().get(url);
    if let Some(r) = referer {
        peticion = peticion.header("Referer", r);
    }
    let mut respuesta = peticion.call().map_err(|e| format!("{url}: {e}"))?;
    respuesta
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e}"))
}

/// Descarga a `destino`, avisando del progreso (bytes, total si se conoce). Devuelve el nombre
/// de fichero que propone el servidor (Content-Disposition), si lo hay. `cancelado` se consulta
/// entre bloques.
pub fn descargar(
    url: &str,
    referer: Option<&str>,
    destino: &Path,
    progreso: &dyn Fn(u64, Option<u64>),
    cancelado: &dyn Fn() -> bool,
) -> Result<Option<String>, String> {
    crate::depurar!("descarga {url} → {}", destino.display());
    let mut peticion = agente().get(url);
    if let Some(r) = referer {
        peticion = peticion.header("Referer", r);
    }
    let respuesta = peticion.call().map_err(|e| format!("{url}: {e}"))?;
    let total = respuesta
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let nombre = respuesta
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .and_then(nombre_disposicion);
    let tipo = respuesta
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if tipo.starts_with("text/html") {
        return Err("la web ha devuelto una página en lugar del vídeo".into());
    }

    let mut lector = respuesta.into_body().into_reader();
    let mut fichero = std::fs::File::create(destino).map_err(|e| e.to_string())?;
    let mut bufer = vec![0u8; 256 * 1024];
    let mut hecho = 0u64;
    loop {
        if cancelado() {
            drop(fichero);
            let _ = std::fs::remove_file(destino);
            return Err("cancelada".into());
        }
        let n = lector.read(&mut bufer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        fichero.write_all(&bufer[..n]).map_err(|e| e.to_string())?;
        hecho += n as u64;
        progreso(hecho, total);
    }
    Ok(nombre)
}

fn nombre_disposicion(valor: &str) -> Option<String> {
    // filename*=UTF-8''nombre tiene preferencia sobre filename="nombre".
    for parte in valor.split(';').map(str::trim) {
        if let Some(n) = parte.strip_prefix("filename*=UTF-8''") {
            return Some(percent_decode(n));
        }
    }
    for parte in valor.split(';').map(str::trim) {
        if let Some(n) = parte.strip_prefix("filename=") {
            return Some(n.trim_matches('"').to_string());
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut salida = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                salida.push(v);
                i += 3;
                continue;
            }
        }
        salida.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&salida).into_owned()
}

pub fn codificar_url(s: &str) -> String {
    let mut salida = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => salida.push(b as char),
            b' ' => salida.push('+'),
            _ => salida.push_str(&format!("%{b:02X}")),
        }
    }
    salida
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn disposicion() {
        assert_eq!(
            nombre_disposicion("attachment; filename=\"a.mp4\"; filename*=UTF-8''b%20c.mp4").as_deref(),
            Some("b c.mp4")
        );
        assert_eq!(nombre_disposicion("attachment; filename=x.mp4").as_deref(), Some("x.mp4"));
        assert_eq!(codificar_url("anime girl"), "anime+girl");
    }
}
