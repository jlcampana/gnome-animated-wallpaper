//! Versiones nuevas: al abrir la app se mira la última release de GitHub y, si es más nueva,
//! sale un aviso arriba. «Actualizar» cierra la app, ejecuta el install.sh de esa versión (sin
//! tocar los paquetes del sistema) y la vuelve a abrir.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::contexto::Contexto;

const REPO: &str = "jlcampana/gnome-animated-wallpaper";
const ACTUAL: &str = env!("CARGO_PKG_VERSION");
/// La app reabierta tras actualizar recibe aquí «ok» o «error».
const VARIABLE: &str = "FONDOS_ANIMADOS_ACTUALIZADA";

// Va desacoplado de la app: ella se cierra y esto sigue. $1 = registro, $2 = versión.
const SCRIPT: &str = r#"set -o pipefail
sleep 2
mkdir -p "$(dirname "$1")"
if curl -fsSL "https://raw.githubusercontent.com/REPO/v$2/install.sh" |
     bash -s -- --si --sin-dependencias >"$1" 2>&1; then r=ok; else r=error; fi
app=$(command -v fondos-animados || echo "$HOME/.local/bin/fondos-animados")
FONDOS_ANIMADOS_ACTUALIZADA=$r exec "$app"
"#;

/// Versión de la última release publicada, sin la «v». Bloqueante.
fn ultima() -> Result<String, String> {
    let json = crate::red::texto(&format!("https://api.github.com/repos/{REPO}/releases/latest"), None)?;
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    v["tag_name"]
        .as_str()
        .map(|t| t.trim_start_matches('v').to_string())
        .ok_or_else(|| "la respuesta no trae tag_name".to_string())
}

fn es_mas_nueva(otra: &str, actual: &str) -> bool {
    let numeros = |v: &str| v.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    numeros(otra) > numeros(actual)
}

fn registro() -> std::path::PathBuf {
    glib::user_cache_dir().join("fondos-animados").join("actualizacion.log")
}

/// Prepara el aviso, enseña el resultado si se viene de actualizar y deja en el contexto la
/// función de comprobar (la usa también Ajustes). Comprueba ya si check-updates está activo.
pub fn preparar(ctx: &Rc<Contexto>, aviso: &adw::Banner) {
    match std::env::var(VARIABLE).as_deref() {
        Ok("ok") => ctx.aviso(&format!(
            "Actualizada a la versión {ACTUAL}. La extensión nueva se carga al volver a entrar en la sesión"
        )),
        Ok(_) => {
            aviso.set_title(&format!("No se pudo actualizar. Detalles en {}", registro().display()));
            aviso.set_button_label(None);
            aviso.set_revealed(true);
        }
        Err(_) => {}
    }
    // La versión que ofrece el aviso, y la que se instala al cerrar la ventana tras «Actualizar».
    let nueva = Rc::new(RefCell::new(String::new()));
    let al_cerrar = Rc::new(RefCell::new(None::<String>));
    {
        let nueva = nueva.clone();
        let al_cerrar = al_cerrar.clone();
        let ventana = ctx.ventana.clone();
        aviso.connect_button_clicked(move |a| {
            crate::depurar!("actualizar a {} al cerrar", nueva.borrow());
            al_cerrar.replace(Some(nueva.borrow().clone()));
            // Con descargas del navegador en marcha, la ventana pregunta antes de cerrarse; si se
            // queda abierta, se actualiza cuando se cierre.
            a.set_title(&format!("Se actualizará a la {} al cerrar la app", nueva.borrow()));
            a.set_button_label(None);
            ventana.close();
        });
    }
    if let Some(app) = ctx.ventana.application() {
        app.connect_shutdown(move |_| {
            if let Some(v) = al_cerrar.borrow().as_deref() {
                actualizar(v);
            }
        });
    }
    let comprobar = {
        let ctx = Rc::downgrade(ctx);
        let aviso = aviso.clone();
        move |manual: bool| {
            let Some(ctx) = ctx.upgrade() else { return };
            let aviso = aviso.clone();
            let nueva = nueva.clone();
            glib::spawn_future_local(async move {
                let r = gio::spawn_blocking(ultima).await.unwrap_or_else(|_| Err("fallo interno".into()));
                match r {
                    Ok(v) if es_mas_nueva(&v, ACTUAL) => {
                        aviso.set_title(&format!("Hay una versión nueva: la {v} (tienes la {ACTUAL})"));
                        aviso.set_button_label(Some("Actualizar"));
                        aviso.set_revealed(true);
                        nueva.replace(v);
                    }
                    Ok(_) if manual => ctx.aviso(&format!("Tienes la última versión ({ACTUAL})")),
                    Err(e) if manual => ctx.aviso(&format!("No se pudo comprobar: {e}")),
                    Err(e) => crate::depurar!("comprobar versión: {e}"),
                    Ok(_) => {}
                }
            });
        }
    };
    if ctx.ajustes.boolean("check-updates") {
        comprobar(false);
    }
    ctx.buscar_actualizacion.replace(Some(Box::new(comprobar)));
}

fn actualizar(version: &str) {
    let script = SCRIPT.replace("REPO", REPO);
    let reg = registro();
    let args = ["bash", "-c", &script, "actualizar", &reg.to_string_lossy(), version];
    let argv: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    match gio::Subprocess::newv(&argv, gio::SubprocessFlags::NONE) {
        Ok(_) => crate::depurar!("actualizando a {version}; registro en {}", reg.display()),
        Err(e) => eprintln!("fondos-animados: no se pudo lanzar la actualización: {e}"),
    }
}

#[cfg(test)]
mod pruebas {
    use super::es_mas_nueva;

    #[test]
    fn compara_versiones() {
        assert!(es_mas_nueva("0.5.0", "0.4.1"));
        assert!(es_mas_nueva("0.4.10", "0.4.9"));
        assert!(es_mas_nueva("1.0", "0.9.9"));
        assert!(!es_mas_nueva("0.4.1", "0.4.1"));
        assert!(!es_mas_nueva("0.4.0", "0.4.1"));
    }
}
