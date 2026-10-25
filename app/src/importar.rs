//! Meter vídeos en la biblioteca: de un fichero suelto, de un .zip/.rar/.7z o de una carpeta de
//! Wallpaper Engine (project.json de tipo «video»). Quita el audio si toca y avisa si el códec
//! no lo puede decodificar la GPU. Todo bloqueante: se llama desde gio::spawn_blocking.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use crate::config;

#[derive(Clone, Debug, Default)]
pub struct InfoVideo {
    pub ancho: u32,
    pub alto: u32,
    pub fps: f64,
    pub duracion: f64,
    pub codec: String,
    pub audio: bool,
    pub tamano: u64,
}

impl InfoVideo {
    pub fn resumen(&self) -> String {
        let mut partes = vec![];
        if self.ancho > 0 {
            partes.push(format!("{}×{}", self.ancho, self.alto));
        }
        if self.fps > 0.0 {
            partes.push(format!("{:.0} fps", self.fps));
        }
        if self.duracion > 0.0 {
            partes.push(format!("{:.1} s", self.duracion));
        }
        if !self.codec.is_empty() {
            partes.push(self.codec.to_uppercase());
        }
        partes.push(config::tamano_legible(self.tamano));
        if self.audio {
            partes.push("con audio".into());
        }
        partes.join(" · ")
    }
}

pub fn info(ruta: &Path) -> Result<InfoVideo, String> {
    let salida = Command::new("ffprobe")
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(ruta)
        .output()
        .map_err(|e| format!("no se pudo ejecutar ffprobe: {e}"))?;
    if !salida.status.success() {
        return Err(format!("{} no parece un vídeo", ruta.display()));
    }
    let json: serde_json::Value = serde_json::from_slice(&salida.stdout).map_err(|e| e.to_string())?;
    let mut info = InfoVideo {
        tamano: std::fs::metadata(ruta).map(|m| m.len()).unwrap_or(0),
        ..Default::default()
    };
    for s in json["streams"].as_array().into_iter().flatten() {
        match s["codec_type"].as_str() {
            Some("video") if info.codec.is_empty() => {
                info.codec = s["codec_name"].as_str().unwrap_or_default().to_string();
                info.ancho = s["width"].as_u64().unwrap_or(0) as u32;
                info.alto = s["height"].as_u64().unwrap_or(0) as u32;
                info.fps = fraccion(s["avg_frame_rate"].as_str().unwrap_or("0/0"))
                    .or_else(|| fraccion(s["r_frame_rate"].as_str().unwrap_or("0/0")))
                    .unwrap_or(0.0);
            }
            Some("audio") => info.audio = true,
            _ => {}
        }
    }
    info.duracion = json["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    if info.codec.is_empty() {
        return Err(format!("{} no tiene pista de vídeo", ruta.display()));
    }
    Ok(info)
}

fn fraccion(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (b > 0.0).then(|| a / b)
}

/// Códecs que la GPU decodifica (de las capacidades detectadas; se calcula una vez).
pub fn decodifica_gpu(codec: &str) -> bool {
    static CODECS: OnceLock<Vec<String>> = OnceLock::new();
    CODECS.get_or_init(|| capacidades(false).decodifica_gpu).iter().any(|c| c == codec)
}

/// Quita el audio sin recodificar (script fondo-animado-sin-audio).
pub fn quitar_audio(ruta: &Path) -> Result<(), String> {
    crate::depurar!("quitar audio: {}", ruta.display());
    let salida = Command::new(config::programa("fondo-animado-sin-audio"))
        .arg(ruta)
        .output()
        .map_err(|e| format!("no se pudo ejecutar fondo-animado-sin-audio: {e}"))?;
    if salida.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&salida.stderr).trim().to_string())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Capacidades {
    pub decodifica_gpu: Vec<String>,
    pub codifica_gpu: Vec<String>,
    pub recomendado: String,
    pub codificador: String,
}

impl Capacidades {
    pub fn codifica_por_gpu(&self) -> bool {
        !self.codificador.starts_with("lib")
    }
}

/// Lo que sabe hacer este equipo (fondo-animado-convertir --capacidades, con caché).
pub fn capacidades(forzar: bool) -> Capacidades {
    let mut c = Command::new(config::programa("fondo-animado-convertir"));
    c.arg("--capacidades");
    if forzar {
        c.arg("--forzar");
    }
    let mut cap = Capacidades::default();
    let Ok(salida) = c.output() else { return cap };
    for linea in String::from_utf8_lossy(&salida.stdout).lines() {
        crate::depurar!("capacidades: {linea}");
        let (clave, valor) = linea.split_once(' ').unwrap_or((linea, ""));
        let lista = || valor.split_whitespace().map(str::to_string).collect();
        match clave {
            "decodifica-gpu" => cap.decodifica_gpu = lista(),
            "codifica-gpu" => cap.codifica_gpu = lista(),
            "recomendado" => cap.recomendado = valor.trim().to_string(),
            "codificador" => cap.codificador = valor.trim().to_string(),
            _ => {}
        }
    }
    cap
}

/// Convierte `origen` en `destino` en el formato recomendado; con `resolucion`, además la adapta
/// (recortando), con `fps`, cambia los fotogramas por segundo, y con `calidad`, usa ese QP/CRF
/// en vez del del script. Si falla, no deja `destino`.
fn convertir(
    origen: &Path,
    destino: &Path,
    resolucion: Option<(i32, i32)>,
    fps: Option<u32>,
    calidad: Option<i32>,
) -> Result<(), String> {
    let mut c = Command::new(config::programa("fondo-animado-convertir"));
    if let Some((w, h)) = resolucion {
        c.args(["-r", &format!("{w}x{h}")]);
    }
    if let Some(f) = fps {
        c.args(["-f", &f.to_string()]);
    }
    if let Some(q) = calidad {
        c.args(["-q", &q.to_string()]);
    }
    crate::depurar!(
        "convertir {} → {} (resolución {resolucion:?}, fps {fps:?}, calidad {calidad:?})",
        origen.display(),
        destino.display()
    );
    let salida = c
        .arg(origen)
        .arg(destino)
        .output()
        .map_err(|e| format!("no se pudo ejecutar fondo-animado-convertir: {e}"))?;
    if !salida.status.success() {
        let _ = std::fs::remove_file(destino);
        return Err(String::from_utf8_lossy(&salida.stderr).trim().to_string());
    }
    Ok(())
}

/// Recodifica en el formato recomendado, sustituyendo al original; con `resolucion`, además la
/// adapta (recortando).
pub fn optimizar(ruta: &Path, resolucion: Option<(i32, i32)>) -> Result<PathBuf, String> {
    let destino = ruta.with_extension("optimizando.mp4");
    convertir(ruta, &destino, resolucion, None, None)?;
    let _ = std::fs::remove_file(ruta);
    let final_ = ruta.with_extension("mp4");
    std::fs::rename(&destino, &final_).map_err(|e| e.to_string())?;
    Ok(final_)
}

/// Fotogramas por segundo por encima de los cuales tiene sentido una versión de 30 (el medio
/// fotograma de margen es para los 30000/1001 y similares).
pub fn mas_de_30_fps(fps: f64) -> bool {
    fps > 30.5
}

/// Opciones de fondo-animado-convertir para la versión de 30 fps, con la `calidad` dada (QP/CRF).
pub fn opciones_version_30(calidad: i32) -> Vec<String> {
    vec!["-f".into(), "30".into(), "-q".into(), calidad.to_string()]
}

/// Lo que va contando fondo-animado-convertir --progreso.
#[derive(Debug, PartialEq)]
pub enum Avance {
    /// Segundos que durará el resultado.
    Duracion(f64),
    /// El codificador de ffmpeg con el que va.
    Codificador(String),
    /// Ha fallado un codificador y prueba con el siguiente (vuelve a empezar).
    Reinicio,
    /// Segundos ya codificados.
    Hecho(f64),
}

/// Lanza fondo-animado-convertir --progreso con `opciones` de `origen` a `destino`. Por cada
/// avance llama a `seguir`; si devuelve false, para la conversión (SIGTERM: el script mata a
/// ffmpeg y borra `destino`). Ok(false) si se paró así.
pub fn convertir_con_progreso(
    origen: &Path,
    destino: &Path,
    opciones: &[String],
    seguir: &mut dyn FnMut(Avance) -> bool,
) -> Result<bool, String> {
    crate::depurar!("convertir {} → {} ({})", origen.display(), destino.display(), opciones.join(" "));
    let mut hijo = Command::new(config::programa("fondo-animado-convertir"))
        .arg("--progreso")
        .args(opciones)
        .arg(origen)
        .arg(destino)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo ejecutar fondo-animado-convertir: {e}"))?;
    // stderr en otro hilo, para que no se llene la tubería mientras se lee stdout.
    let mut stderr = hijo.stderr.take().expect("stderr");
    let errores = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut parado = false;
    for linea in BufReader::new(hijo.stdout.take().expect("stdout")).lines() {
        let Ok(linea) = linea else { break };
        let linea = linea.trim();
        let avance = if let Some(d) = linea.strip_prefix("DURACION ") {
            Avance::Duracion(d.parse().unwrap_or(0.0))
        } else if let Some(c) = linea.strip_prefix("CODIFICADOR ") {
            Avance::Codificador(c.to_string())
        } else if linea == "REINICIO" {
            Avance::Reinicio
        } else if let Some(us) = linea.strip_prefix("out_time_us=") {
            match us.parse::<f64>() {
                Ok(us) if us >= 0.0 => Avance::Hecho(us / 1e6),
                _ => continue,
            }
        } else {
            continue;
        };
        if !parado && !seguir(avance) {
            parado = true;
            let _ = Command::new("kill").arg("-TERM").arg(hijo.id().to_string()).status();
        }
    }
    let estado = hijo.wait().map_err(|e| e.to_string())?;
    let errores = errores.join().unwrap_or_default();
    if parado {
        let _ = std::fs::remove_file(destino);
        return Ok(false);
    }
    if !estado.success() {
        let _ = std::fs::remove_file(destino);
        let ultima = errores.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("la conversión ha fallado");
        return Err(ultima.trim().to_string());
    }
    Ok(true)
}

const ARCHIVOS: &[&str] = &["zip", "rar", "7z", "tar", "gz", "xz"];

fn es_archivo_comprimido(ruta: &Path) -> bool {
    ruta.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ARCHIVOS.contains(&e.to_lowercase().as_str()))
}

fn extraer(archivo: &Path, destino: &Path) -> Result<(), String> {
    std::fs::create_dir_all(destino).map_err(|e| e.to_string())?;
    let mut salida_dir = std::ffi::OsString::from("-o");
    salida_dir.push(destino);
    let intentos: Vec<Command> = vec![
        {
            let mut c = Command::new("7z");
            c.arg("x").arg("-y").arg(&salida_dir).arg(archivo);
            c
        },
        {
            let mut c = Command::new("unzip");
            c.arg("-o").arg(archivo).arg("-d").arg(destino);
            c
        },
        {
            let mut c = Command::new("unrar");
            c.arg("x").arg("-o+").arg(archivo).arg(destino);
            c
        },
        {
            let mut c = Command::new("bsdtar");
            c.arg("-xf").arg(archivo).arg("-C").arg(destino);
            c
        },
    ];
    for mut c in intentos {
        if c.output().is_ok_and(|o| o.status.success()) {
            return Ok(());
        }
    }
    Err("no se pudo descomprimir (instala p7zip, unzip o unrar)".into())
}

/// En una carpeta: el vídeo de un project.json de Wallpaper Engine, o el vídeo más grande.
/// Devuelve también el título si lo hay.
fn buscar_video(dir: &Path) -> Result<(PathBuf, Option<String>), String> {
    let mut proyectos = vec![];
    let mut videos = vec![];
    let mut pila = vec![dir.to_path_buf()];
    while let Some(d) = pila.pop() {
        let Ok(entradas) = std::fs::read_dir(&d) else { continue };
        for e in entradas.flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
            } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("project.json")) {
                proyectos.push(p);
            } else if config::es_video(&p) {
                videos.push(p);
            }
        }
    }

    let mut tipo_no_compatible = None;
    for proyecto in proyectos {
        let Ok(texto) = std::fs::read_to_string(&proyecto) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&texto) else { continue };
        let tipo = json["type"].as_str().unwrap_or_default().to_lowercase();
        let titulo = json["title"].as_str().map(str::to_string);
        if tipo == "video" {
            if let Some(f) = json["file"].as_str() {
                let video = proyecto.parent().unwrap_or(dir).join(f);
                if video.is_file() {
                    return Ok((video, titulo));
                }
            }
        } else if !tipo.is_empty() {
            tipo_no_compatible = Some(tipo);
        }
    }

    videos
        .into_iter()
        .max_by_key(|v| std::fs::metadata(v).map(|m| m.len()).unwrap_or(0))
        .map(|v| (v, None))
        .ok_or_else(|| match tipo_no_compatible {
            Some(t) => format!(
                "es un fondo de Wallpaper Engine de tipo «{t}»; solo se pueden importar los de tipo vídeo"
            ),
            None => "no hay ningún vídeo dentro".into(),
        })
}

#[derive(Clone, Debug)]
pub struct Resultado {
    pub ruta: PathBuf,
    pub info: InfoVideo,
    pub avisos: Vec<String>,
}

/// Importa `origen` (vídeo, archivo comprimido o carpeta) a la biblioteca. `mover`: el origen
/// es temporal (descarga) y se puede mover en vez de copiar. La versión de 30 fps, si toca, la
/// encola luego el gestor (es una conversión más).
pub fn importar(
    origen: &Path,
    biblioteca: &Path,
    titulo: Option<&str>,
    sin_audio: bool,
    mover: bool,
    optimizar_a: Option<(i32, i32)>,
) -> Result<Resultado, String> {
    crate::depurar!("importar {} (sin audio: {sin_audio}, optimizar: {optimizar_a:?})", origen.display());
    std::fs::create_dir_all(biblioteca).map_err(|e| e.to_string())?;
    let mut temporal = None;
    let (video, titulo_paquete, mut mover) = if origen.is_dir() {
        let (v, t) = buscar_video(origen)?;
        (v, t, false)
    } else if es_archivo_comprimido(origen) {
        let dir = config::carpeta_cache().join(format!("extraer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        extraer(origen, &dir)?;
        temporal = Some(dir.clone());
        let (v, t) = buscar_video(&dir)?;
        (v, t, true)
    } else {
        (origen.to_path_buf(), None, mover)
    };

    let base = titulo
        .map(str::to_string)
        .or(titulo_paquete)
        .unwrap_or_else(|| video.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    // Los nombres de descarga traen «-moewalls-com» y similares: fuera.
    let mut base = config::nombre_limpio(&base);
    for sufijo in ["-moewalls-com", "-wallpaperwaifu-com", "-motionbgs-com", "-1920x1080", "-3840x2160"] {
        if let Some(b) = base.strip_suffix(sufijo) {
            base = b.to_string();
        }
    }
    let ext = video.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_lowercase();
    let destino = config::ruta_libre(biblioteca, &base, &ext);

    // Si ya está dentro de la biblioteca, no se copia.
    if video.parent() == Some(biblioteca) {
        mover = true;
    }
    if mover {
        if std::fs::rename(&video, &destino).is_err() {
            std::fs::copy(&video, &destino).map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(&video);
        }
    } else {
        std::fs::copy(&video, &destino).map_err(|e| e.to_string())?;
    }
    if let Some(dir) = temporal {
        let _ = std::fs::remove_dir_all(dir);
    }

    let mut ruta = destino;
    let mut info = info(&ruta)?;
    let mut avisos = vec![];
    if sin_audio && info.audio {
        match quitar_audio(&ruta) {
            Ok(()) => info = self::info(&ruta)?,
            Err(e) => avisos.push(format!("no se pudo quitar el audio: {e}")),
        }
    }
    // Adaptar a la pantalla (si se pidió y el vídeo es más grande o de otra forma), o al menos
    // pasar a un códec que la GPU decodifique.
    let distinto = optimizar_a.is_some_and(|(w, h)| info.ancho as i32 != w || info.alto as i32 != h);
    if distinto || !decodifica_gpu(&info.codec) {
        let codec = info.codec.to_uppercase();
        let res = if distinto { optimizar_a } else { None };
        match optimizar(&ruta, res) {
            Ok(nueva) => {
                ruta = nueva;
                let antes = info.tamano;
                info = self::info(&ruta)?;
                avisos.push(if distinto {
                    format!(
                        "adaptado a {}×{} en {} ({} → {})",
                        info.ancho,
                        info.alto,
                        info.codec.to_uppercase(),
                        config::tamano_legible(antes),
                        config::tamano_legible(info.tamano)
                    )
                } else {
                    format!("convertido de {codec} a {}: la GPU no decodifica {codec}", info.codec.to_uppercase())
                });
            }
            Err(e) => avisos.push(format!("no se pudo convertir: {e}")),
        }
    }
    Ok(Resultado { ruta, info, avisos })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::fuentes::Fuente;

    fn dir_prueba(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fondos-prueba-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Descarga real de WallpaperWaifu e importación (con audio añadido, que se tiene que ir).
    /// cargo test -- --ignored
    #[test]
    #[ignore]
    fn descarga_e_importacion_reales() {
        let f = Fuente::WallpaperWaifu;
        let r = f.buscar("", 1, Some("1920x1080"), 1).unwrap();
        let primero = r.first().expect("resultados");
        let det = f.detalle(primero).unwrap();
        let dir = dir_prueba("descarga");
        let bajado = dir.join("bajado.mp4");
        let nombre = red_descargar(&det.descargas[0].url, &primero.pagina, &bajado);
        println!("descargado {:?} ({} bytes)", nombre, std::fs::metadata(&bajado).unwrap().len());

        // Le añadimos audio para comprobar que se quita.
        let con_audio = dir.join("con-audio.mp4");
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-i"])
            .arg(&bajado)
            .args(["-f", "lavfi", "-i", "sine=d=3", "-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-shortest"])
            .arg(&con_audio)
            .status()
            .unwrap();
        assert!(ok.success());
        assert!(info(&con_audio).unwrap().audio);

        let biblioteca = dir.join("biblioteca");
        let res = importar(&con_audio, &biblioteca, Some(&primero.titulo), true, true, None).unwrap();
        println!("importado {} · {} · avisos {:?}", res.ruta.display(), res.info.resumen(), res.avisos);
        assert!(!res.info.audio);
        assert!(res.ruta.starts_with(&biblioteca));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn video_60_fps(dir: &Path, segundos: u32) -> PathBuf {
        let video = dir.join("Rápido.mp4");
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc=s=320x180:r=60:d={segundos}")])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&video)
            .status()
            .unwrap();
        assert!(ok.success());
        video
    }

    /// La versión de 30 fps con progreso: cuenta la duración, el codificador y el avance.
    #[test]
    fn version_de_30_fps_con_progreso() {
        let dir = dir_prueba("v30");
        let video = video_60_fps(&dir, 1);
        let destino = dir.join("30.mp4");
        let mut avances = vec![];
        let r = convertir_con_progreso(&video, &destino, &opciones_version_30(28), &mut |a| {
            avances.push(a);
            true
        });
        assert_eq!(r, Ok(true));
        assert_eq!(info(&destino).unwrap().fps.round(), 30.0);
        assert!(avances.contains(&Avance::Duracion(1.0)), "{avances:?}");
        assert!(avances.iter().any(|a| matches!(a, Avance::Codificador(_))));
        assert!(avances.iter().any(|a| matches!(a, Avance::Hecho(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Parar a mitad: el script mata a ffmpeg y no queda el fichero a medias.
    #[test]
    fn conversion_cancelada() {
        let dir = dir_prueba("cancelar");
        let video = video_60_fps(&dir, 20);
        let destino = dir.join("30.mp4");
        let opciones = ["-f", "30", "-c", "hevc", "--cpu"].map(String::from);
        let r = convertir_con_progreso(&video, &destino, &opciones, &mut |a| !matches!(a, Avance::Hecho(_)));
        assert_eq!(r, Ok(false));
        assert!(!destino.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn red_descargar(url: &str, referer: &str, destino: &Path) -> Option<String> {
        crate::red::descargar(url, Some(referer), destino, &|_, _| {}, &|| false).unwrap()
    }

    /// Paquetes de Wallpaper Engine: el de vídeo se importa con su título; el de escena, no.
    #[test]
    fn paquetes_wallpaper_engine() {
        let dir = dir_prueba("we");
        let paquete = dir.join("paquete");
        std::fs::create_dir_all(&paquete).unwrap();
        let ok = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=s=320x180:d=1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(paquete.join("fondo.mp4"))
            .status()
            .unwrap();
        assert!(ok.success());
        std::fs::write(paquete.join("project.json"), r#"{"title":"Mi Fondo WE","type":"Video","file":"fondo.mp4"}"#).unwrap();
        let zip = dir.join("paquete.zip");
        assert!(Command::new("7z").arg("a").arg(&zip).arg(&paquete).output().unwrap().status.success());

        let biblioteca = dir.join("biblioteca");
        let r = importar(&zip, &biblioteca, None, true, false, None).unwrap();
        assert_eq!(r.ruta.file_name().unwrap(), "mi-fondo-we.mp4");

        std::fs::write(paquete.join("project.json"), r#"{"title":"Escena","type":"scene","file":"scene.json"}"#).unwrap();
        std::fs::remove_file(paquete.join("fondo.mp4")).unwrap();
        let e = importar(&paquete, &biblioteca, None, true, false, None).unwrap_err();
        assert!(e.contains("scene"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
