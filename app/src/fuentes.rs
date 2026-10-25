//! Webs de fondos animados: cómo se busca en cada una, cómo se leen sus listados y de dónde
//! sale el vídeo. Todo bloqueante (se usa desde hilos aparte).
//!
//! - motionbgs.com: listados /N/, /tag:x/N/, /search?q=…&page=N; descarga directa
//!   /dl/hd/<id>/ (1920x1080) o /dl/4k/<id>/.
//! - moewalls.com y wallpaperwaifu.com (mismo motor WordPress): /page/N/?s=…, /category/x/,
//!   /resolution/WxH/; el botón de descarga lleva data-url, que va a
//!   https://go.<web>/download.php?video=<data-url>.
//! - vsthemes.org: tiene comprobación anti-bots con captcha; se usa con el navegador integrado.

use scraper::{Html, Selector};

use crate::red;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fuente {
    MotionBgs,
    MoeWalls,
    WallpaperWaifu,
    VsThemes,
}

#[derive(Clone, Debug)]
pub struct Resultado {
    pub titulo: String,
    pub pagina: String,
    pub miniatura: String,
    pub resolucion: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Descarga {
    pub etiqueta: String,
    pub url: String,
}

#[derive(Clone, Debug, Default)]
pub struct Detalle {
    pub vista_previa: Option<String>,
    pub descargas: Vec<Descarga>,
}

pub const RESOLUCIONES: &[&str] = &["1920x1080", "2560x1080", "2560x1440", "3440x1440", "3840x2160"];

impl Fuente {
    pub const TODAS: [Fuente; 4] = [Fuente::MotionBgs, Fuente::MoeWalls, Fuente::WallpaperWaifu, Fuente::VsThemes];

    pub fn nombre(self) -> &'static str {
        match self {
            Fuente::MotionBgs => "MotionBGs",
            Fuente::MoeWalls => "MoeWalls",
            Fuente::WallpaperWaifu => "WallpaperWaifu",
            Fuente::VsThemes => "VSThemes (navegador)",
        }
    }

    pub fn web(self) -> &'static str {
        match self {
            Fuente::MotionBgs => "https://motionbgs.com",
            Fuente::MoeWalls => "https://moewalls.com",
            Fuente::WallpaperWaifu => "https://wallpaperwaifu.com",
            Fuente::VsThemes => "https://vsthemes.org",
        }
    }

    /// Se puede buscar y descargar desde la app; si no, va por el navegador integrado.
    pub fn nativa(self) -> bool {
        self != Fuente::VsThemes
    }

    /// (etiqueta, ruta) de las categorías. La primera es «Recientes».
    pub fn categorias(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Fuente::MotionBgs => &[
                ("Recientes", "/"),
                ("Anime", "/tag:anime/"),
                ("Chicas", "/tag:girl/"),
                ("Juegos", "/tag:games/"),
                ("Fantasía", "/tag:fantasy/"),
                ("Naturaleza", "/tag:nature/"),
                ("Espacio", "/tag:space/"),
                ("Lluvia", "/tag:rain/"),
                ("Japón", "/tag:japan/"),
                ("Gatos", "/tag:cat/"),
                ("Coches", "/tag:car/"),
                ("Solo 4K", "/4k/"),
            ],
            Fuente::MoeWalls => &[
                ("Recientes", "/"),
                ("Anime", "/category/anime/"),
                ("Chicas anime", "/tag/anime-girl/"),
                ("Juegos", "/category/games/"),
                ("Fantasía", "/category/fantasy/"),
                ("Paisajes", "/category/landscape/"),
                ("Pixel art", "/category/pixel-art/"),
                ("Ciencia ficción", "/category/sci-fi/"),
                ("Películas", "/category/movies/"),
                ("Animales", "/category/animal/"),
                ("Abstractos", "/category/abstract/"),
            ],
            Fuente::WallpaperWaifu => &[
                ("Recientes", "/"),
                ("Anime", "/category/anime/"),
                ("Juegos", "/category/games/"),
                ("Fantasía", "/category/fantasy/"),
                ("Paisajes", "/category/landscape/"),
                ("Pixel art", "/category/pixel-art/"),
                ("Ciencia ficción", "/category/sci-fi/"),
                ("Películas", "/category/movies/"),
                ("Animales", "/category/animals/"),
                ("Abstractos", "/category/abstract/"),
            ],
            Fuente::VsThemes => &[
                ("Anime", "/en/wallpapers/anime/"),
                ("Todos los fondos", "/en/wallpapers/"),
            ],
        }
    }

    /// Si la web sabe filtrar por resolución (motionbgs ofrece HD y 4K de todo).
    pub fn filtra_resolucion(self) -> bool {
        matches!(self, Fuente::MoeWalls | Fuente::WallpaperWaifu)
    }

    /// URL de una página de resultados. `consulta` vacía: se usa la categoría.
    pub fn url_listado(self, consulta: &str, categoria: usize, resolucion: Option<&str>, pagina: u32) -> String {
        let web = self.web();
        let consulta = consulta.trim();
        let cat = self.categorias().get(categoria).map(|c| c.1).unwrap_or("/");
        match self {
            Fuente::MotionBgs => {
                if !consulta.is_empty() {
                    let q = red::codificar_url(consulta);
                    if pagina > 1 { format!("{web}/search?q={q}&page={pagina}") } else { format!("{web}/search?q={q}") }
                } else if pagina > 1 {
                    format!("{web}{cat}{pagina}/")
                } else {
                    format!("{web}{cat}")
                }
            }
            Fuente::MoeWalls | Fuente::WallpaperWaifu => {
                let pag = if pagina > 1 { format!("page/{pagina}/") } else { String::new() };
                if !consulta.is_empty() {
                    format!("{web}/{pag}?s={}", red::codificar_url(consulta))
                } else if let (Fuente::MoeWalls, Some(r), "/") = (self, resolucion, cat) {
                    // moewalls solo filtra por resolución con su propia sección.
                    format!("{web}/resolution/{r}/{pag}")
                } else {
                    let mut url = format!("{web}{cat}{pag}");
                    if let (Fuente::WallpaperWaifu, Some(r)) = (self, resolucion) {
                        url.push_str(&format!("?resolution={r}"));
                    }
                    url
                }
            }
            Fuente::VsThemes => {
                if consulta.is_empty() {
                    format!("{web}{cat}")
                } else {
                    format!("{web}/en/search/?q={}", red::codificar_url(consulta))
                }
            }
        }
    }

    pub fn analizar_listado(self, html: &str) -> Vec<Resultado> {
        let doc = Html::parse_document(html);
        let mut resultados = match self {
            Fuente::MotionBgs => listado_motionbgs(&doc, self.web()),
            Fuente::MoeWalls | Fuente::WallpaperWaifu => listado_wordpress(&doc, self.web()),
            Fuente::VsThemes => vec![],
        };
        // Sin repetidos (algunas webs repiten destacados).
        let mut vistos = std::collections::HashSet::new();
        resultados.retain(|r| vistos.insert(r.pagina.clone()));
        resultados
    }

    pub fn buscar(self, consulta: &str, categoria: usize, resolucion: Option<&str>, pagina: u32) -> Result<Vec<Resultado>, String> {
        let url = self.url_listado(consulta, categoria, resolucion, pagina);
        let html = match red::texto(&url, Some(self.web())) {
            Ok(h) => h,
            // Pasada la última página, las webs dan 404: no hay más resultados.
            Err(e) if pagina > 1 && e.contains("404") => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut r = self.analizar_listado(&html);
        // Con búsqueda de texto la web no filtra por resolución: se filtra aquí.
        if let Some(res) = resolucion {
            if self.filtra_resolucion() {
                r.retain(|x| x.resolucion.as_deref().is_none_or(|v| v == res));
            }
        }
        Ok(r)
    }

    pub fn detalle(self, resultado: &Resultado) -> Result<Detalle, String> {
        let html = red::texto(&resultado.pagina, Some(self.web()))?;
        match self {
            Fuente::MotionBgs => Ok(detalle_motionbgs(&html, self.web())),
            Fuente::MoeWalls | Fuente::WallpaperWaifu => detalle_wordpress(&html, self.web(), resultado.resolucion.as_deref()),
            Fuente::VsThemes => Err("vsthemes va por el navegador".into()),
        }
    }
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("selector")
}

fn absoluta(web: &str, url: &str) -> String {
    let url = url.trim();
    if url.starts_with("http") {
        url.to_string()
    } else if url.starts_with("//") {
        format!("https:{url}")
    } else if url.starts_with('/') {
        format!("{web}{url}")
    } else {
        format!("{web}/{url}")
    }
}

fn limpiar_titulo(t: &str) -> String {
    let t = t.trim();
    for sufijo in [" Live Wallpaper", " live wallpaper", " Live wallpaper"] {
        if let Some(x) = t.strip_suffix(sufijo) {
            return x.trim().to_string();
        }
    }
    t.to_string()
}

/// «…/slug.3840x2160.jpg» → «3840x2160».
fn resolucion_en(url: &str) -> Option<String> {
    let nombre = url.rsplit('/').next()?;
    nombre.split('.').find(|p| {
        p.split_once('x').is_some_and(|(a, b)| {
            a.len() >= 3 && b.len() >= 3 && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit())
        })
    }).map(str::to_string)
}

fn listado_motionbgs(doc: &Html, web: &str) -> Vec<Resultado> {
    let enlaces = sel("a[title][href]");
    let img = sel("img");
    let ttl = sel("span.ttl");
    let mut r = vec![];
    for a in doc.select(&enlaces) {
        let Some(imagen) = a.select(&img).next() else { continue };
        let Some(src) = imagen.value().attr("src") else { continue };
        if !src.contains("/media/") {
            continue;
        }
        let href = a.value().attr("href").unwrap_or_default();
        let titulo = a
            .select(&ttl)
            .next()
            .map(|t| t.text().collect::<String>())
            .unwrap_or_else(|| a.value().attr("title").unwrap_or_default().to_string());
        r.push(Resultado {
            titulo: limpiar_titulo(&titulo),
            pagina: absoluta(web, href),
            miniatura: absoluta(web, src),
            resolucion: resolucion_en(src),
        });
    }
    r
}

fn listado_wordpress(doc: &Html, web: &str) -> Vec<Resultado> {
    let articulos = sel("article");
    let enlace = sel("a[href][title]");
    let img = sel("img");
    let mut r = vec![];
    for art in doc.select(&articulos) {
        let Some(a) = art.select(&enlace).next() else { continue };
        let Some(imagen) = art.select(&img).next() else { continue };
        let src = imagen
            .value()
            .attr("data-src")
            .or_else(|| imagen.value().attr("src"))
            .unwrap_or_default();
        if src.is_empty() || src.starts_with("data:") {
            continue;
        }
        let resolucion = art
            .value()
            .classes()
            .find_map(|c| c.strip_prefix("resolutions-"))
            .map(str::to_string);
        r.push(Resultado {
            titulo: limpiar_titulo(a.value().attr("title").unwrap_or_default()),
            pagina: absoluta(web, a.value().attr("href").unwrap_or_default()),
            miniatura: absoluta(web, src),
            resolucion,
        });
    }
    r
}

/// Primera URL que acaba en una de `extensiones`, buscada en el HTML crudo (las webs la meten
/// a veces en JSON con «\/»).
fn buscar_url(html: &str, extensiones: &[&str]) -> Option<String> {
    let html = html.replace("\\/", "/");
    for ext in extensiones {
        let mut desde = 0;
        while let Some(pos) = html[desde..].find(ext) {
            let fin = desde + pos + ext.len();
            let inicio = html[..fin]
                .rfind(|c: char| c == '"' || c == '\'' || c == '=' || c == ' ' || c == '(')
                .map(|i| i + 1)
                .unwrap_or(0);
            let candidata = &html[inicio..fin];
            if !candidata.is_empty() && (candidata.starts_with('/') || candidata.starts_with("http")) {
                return Some(candidata.to_string());
            }
            desde = fin;
        }
    }
    None
}

fn detalle_motionbgs(html: &str, web: &str) -> Detalle {
    let doc = Html::parse_document(html);
    let mut descargas = vec![];
    for a in doc.select(&sel("a[href^='/dl/']")) {
        let href = a.value().attr("href").unwrap_or_default();
        let etiqueta = if href.contains("/4k/") { "4K (3840x2160)" } else { "HD (1920x1080)" };
        if !descargas.iter().any(|d: &Descarga| d.etiqueta == etiqueta) {
            descargas.push(Descarga { etiqueta: etiqueta.into(), url: absoluta(web, href) });
        }
    }
    Detalle {
        vista_previa: buscar_url(html, &[".mp4"]).map(|u| absoluta(web, &u)),
        descargas,
    }
}

fn detalle_wordpress(html: &str, web: &str, resolucion: Option<&str>) -> Result<Detalle, String> {
    let doc = Html::parse_document(html);
    // El botón de descarga es #moe-download / #wf-download; otros enlaces (compartir en redes)
    // también llevan data-url.
    let clave = doc
        .select(&sel("a[id$='-download'][data-url], a.btndownload[data-url]"))
        .filter_map(|a| a.value().attr("data-url"))
        .find(|u| !u.is_empty() && !u.starts_with("http"))
        .ok_or("no encuentro el enlace de descarga en la página")?;
    let dominio = web.trim_start_matches("https://");
    let etiqueta = match resolucion {
        Some(r) => format!("Original ({r})"),
        None => "Original".into(),
    };
    Ok(Detalle {
        vista_previa: buscar_url(html, &[".webm", ".mp4"]).map(|u| absoluta(web, &u)),
        descargas: vec![Descarga { etiqueta, url: format!("https://go.{dominio}/download.php?video={clave}") }],
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(Fuente::MotionBgs.url_listado("", 2, None, 2), "https://motionbgs.com/tag:girl/2/");
        assert_eq!(Fuente::MotionBgs.url_listado("frieren", 0, None, 2), "https://motionbgs.com/search?q=frieren&page=2");
        assert_eq!(Fuente::MoeWalls.url_listado("", 0, Some("2560x1080"), 3), "https://moewalls.com/resolution/2560x1080/page/3/");
        assert_eq!(Fuente::MoeWalls.url_listado("frieren", 1, None, 2), "https://moewalls.com/page/2/?s=frieren");
        assert_eq!(
            Fuente::WallpaperWaifu.url_listado("", 1, Some("1920x1080"), 2),
            "https://wallpaperwaifu.com/category/anime/page/2/?resolution=1920x1080"
        );
    }

    #[test]
    fn resoluciones() {
        assert_eq!(resolucion_en("/i/c/364x205/media/10319/yamada-tayama.3840x2160.jpg").as_deref(), Some("3840x2160"));
        assert_eq!(limpiar_titulo("Yamada & Tayama Live Wallpaper"), "Yamada & Tayama");
    }

    #[test]
    fn url_en_json() {
        let html = r#"{"src":"\/wp-content\/uploads\/preview\/2026\/x-preview.webm"}"#;
        assert_eq!(buscar_url(html, &[".webm"]).as_deref(), Some("/wp-content/uploads/preview/2026/x-preview.webm"));
    }
}
