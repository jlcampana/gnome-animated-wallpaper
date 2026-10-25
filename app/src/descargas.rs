//! Descargas en curso: un botón en la barra de título con su lista (barra de progreso y
//! cancelar). Al terminar, el fichero se importa a la biblioteca (sin audio, y convertido si la
//! GPU no puede con su códec).

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use adw::prelude::*;
use gtk::{gio, glib};

use crate::contexto::Contexto;
use crate::{config, importar, red};

pub struct Descargas {
    ctx: Rc<Contexto>,
    pub boton: gtk::MenuButton,
    lista: gtk::ListBox,
    activas: Cell<u32>,
}

enum Evento {
    Progreso(u64, Option<u64>),
    Fin(Result<(PathBuf, Option<String>), String>),
}

impl Descargas {
    pub fn new(ctx: Rc<Contexto>) -> Rc<Self> {
        let lista = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let marco = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(400)
            .min_content_width(360)
            .child(&lista)
            .build();
        let popover = gtk::Popover::builder().child(&marco).build();
        let boton = gtk::MenuButton::builder()
            .icon_name("folder-download-symbolic")
            .tooltip_text("Descargas")
            .popover(&popover)
            .visible(false)
            .build();
        Rc::new(Self { ctx, boton, lista, activas: Cell::new(0) })
    }

    fn fila(&self, titulo: &str) -> (gtk::Box, gtk::ProgressBar, gtk::Label, gtk::Button) {
        let nombre = gtk::Label::builder()
            .label(titulo)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(36)
            .build();
        let estado = gtk::Label::builder().label("Empezando…").xalign(0.0).css_classes(["dim-label", "caption"]).build();
        let barra = gtk::ProgressBar::new();
        let cancelar = gtk::Button::builder()
            .icon_name("process-stop-symbolic")
            .tooltip_text("Cancelar")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        let textos = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(4).hexpand(true).build();
        textos.append(&nombre);
        textos.append(&barra);
        textos.append(&estado);
        let caja = gtk::Box::builder()
            .spacing(8)
            .margin_top(8)
            .margin_bottom(8)
            .margin_start(10)
            .margin_end(6)
            .build();
        caja.append(&textos);
        caja.append(&cancelar);
        self.lista.prepend(&caja);
        self.activas.set(self.activas.get() + 1);
        self.boton.set_visible(true);
        (caja, barra, estado, cancelar)
    }

    fn terminar_fila(self: &Rc<Self>, caja: &gtk::Box, cancelar: &gtk::Button) {
        cancelar.set_visible(false);
        self.activas.set(self.activas.get().saturating_sub(1));
        // La fila se queda un rato para ver cómo acabó.
        let yo = self.clone();
        let caja = caja.clone();
        glib::timeout_add_seconds_local_once(20, move || {
            if let Some(fila) = caja.parent() {
                yo.lista.remove(&fila);
            }
            if yo.lista.first_child().is_none() {
                yo.boton.set_visible(false);
            }
        });
    }

    /// Descarga directa (fuentes nativas).
    pub fn descargar(self: &Rc<Self>, titulo: &str, url: &str, referer: &str, usar_al_terminar: bool) {
        let (caja, barra, estado, cancelar) = self.fila(titulo);
        self.ctx.aviso(&format!("Descargando «{titulo}»…"));

        let cancelado = Arc::new(AtomicBool::new(false));
        {
            let cancelado = cancelado.clone();
            cancelar.connect_clicked(move |_| cancelado.store(true, Ordering::Relaxed));
        }

        let dir = config::carpeta_cache().join("descargas");
        let _ = std::fs::create_dir_all(&dir);
        let temporal = dir.join(format!("{}.part", config::nombre_limpio(titulo)));
        let (tx, rx) = async_channel::unbounded::<Evento>();
        {
            let url = url.to_string();
            let referer = referer.to_string();
            let temporal = temporal.clone();
            std::thread::spawn(move || {
                let ultimo = Cell::new(std::time::Instant::now());
                let r = red::descargar(
                    &url,
                    Some(&referer),
                    &temporal,
                    &|hecho, total| {
                        if ultimo.get().elapsed().as_millis() > 150 {
                            ultimo.set(std::time::Instant::now());
                            let _ = tx.send_blocking(Evento::Progreso(hecho, total));
                        }
                    },
                    &|| cancelado.load(Ordering::Relaxed),
                );
                let _ = tx.send_blocking(Evento::Fin(r.map(|nombre| (temporal, nombre))));
            });
        }

        let yo = self.clone();
        let titulo = titulo.to_string();
        glib::spawn_future_local(async move {
            while let Ok(evento) = rx.recv().await {
                match evento {
                    Evento::Progreso(hecho, total) => match total {
                        Some(t) if t > 0 => {
                            barra.set_fraction(hecho as f64 / t as f64);
                            estado.set_label(&format!(
                                "{} de {}",
                                config::tamano_legible(hecho),
                                config::tamano_legible(t)
                            ));
                        }
                        _ => {
                            barra.pulse();
                            estado.set_label(&config::tamano_legible(hecho));
                        }
                    },
                    Evento::Fin(Err(e)) => {
                        estado.set_label(&format!("Error: {e}"));
                        if e != "cancelada" {
                            yo.ctx.aviso(&format!("No se pudo descargar «{titulo}»: {e}"));
                        }
                        yo.terminar_fila(&caja, &cancelar);
                        return;
                    }
                    Evento::Fin(Ok((temporal, nombre))) => {
                        barra.set_fraction(1.0);
                        // Con la extensión que dice el servidor (puede ser .zip).
                        let ext = nombre
                            .as_deref()
                            .and_then(|n| n.rsplit_once('.').map(|(_, e)| e.to_lowercase()))
                            .unwrap_or_else(|| "mp4".into());
                        let fichero = temporal.with_extension(&ext);
                        let _ = std::fs::rename(&temporal, &fichero);
                        yo.importar_fichero(fichero, Some(titulo.clone()), usar_al_terminar, &estado).await;
                        yo.terminar_fila(&caja, &cancelar);
                        return;
                    }
                }
            }
        });
    }

    async fn importar_fichero(
        self: &Rc<Self>,
        fichero: PathBuf,
        titulo: Option<String>,
        usar: bool,
        estado: &gtk::Label,
    ) {
        let optimizar = self.ctx.optimizar_descargas();
        let version_30 = self.ctx.version_30_al_importar();
        estado.set_label(match optimizar {
            Some(_) => "Adaptando el vídeo a la pantalla (puede tardar)…",
            None => "Preparando el vídeo (quitando audio, comprobando códec)…",
        });
        let biblioteca = self.ctx.biblioteca();
        let sin_audio = self.ctx.ajustes.boolean("strip-audio");
        let r = gio::spawn_blocking(move || {
            importar::importar(&fichero, &biblioteca, titulo.as_deref(), sin_audio, true, optimizar, version_30)
        })
        .await
        .unwrap_or_else(|_| Err("fallo interno".into()));
        match r {
            Ok(r) => {
                estado.set_label(&format!("Listo · {}", r.info.resumen()));
                self.ctx.biblioteca_cambiada();
                if usar {
                    self.ctx.usar_video(&r.ruta);
                }
                self.ctx.aviso_importado(&r.ruta, &r.avisos);
            }
            Err(e) => {
                estado.set_label(&format!("Error: {e}"));
                self.ctx.aviso(&format!("No se pudo importar: {e}"));
            }
        }
    }

    /// Descarga hecha por el navegador integrado (WebKit).
    pub fn seguir_webkit(self: &Rc<Self>, descarga: &webkit6::Download) {
        let dir = config::carpeta_cache().join("descargas");
        let _ = std::fs::create_dir_all(&dir);
        let (caja, barra, estado, cancelar) = self.fila("Descarga del navegador");
        {
            let d = descarga.clone();
            cancelar.connect_clicked(move |_| d.cancel());
        }
        let nombre_fila = caja.first_child().and_then(|t| t.first_child()).and_downcast::<gtk::Label>();
        descarga.connect_decide_destination(move |d, sugerido| {
            if let Some(n) = &nombre_fila {
                n.set_label(sugerido);
            }
            let destino = config::ruta_libre(&dir, &config::nombre_limpio(sugerido.rsplit_once('.').map(|x| x.0).unwrap_or(sugerido)),
                sugerido.rsplit_once('.').map(|x| x.1).unwrap_or("bin"));
            d.set_destination(&destino.to_string_lossy());
            true
        });
        {
            let barra = barra.clone();
            let estado = estado.clone();
            descarga.connect_estimated_progress_notify(move |d| {
                barra.set_fraction(d.estimated_progress());
                estado.set_label(&format!("{} descargado", config::tamano_legible(d.received_data_length())));
            });
        }
        let fallo = Rc::new(Cell::new(false));
        {
            let fallo = fallo.clone();
            let estado = estado.clone();
            descarga.connect_failed(move |_, error| {
                fallo.set(true);
                estado.set_label(&format!("Error: {error}"));
            });
        }
        let yo = self.clone();
        descarga.connect_finished(move |d| {
            if fallo.get() {
                yo.terminar_fila(&caja, &cancelar);
                return;
            }
            let Some(destino) = d.destination() else { return };
            let fichero = PathBuf::from(destino.as_str());
            let yo = yo.clone();
            let caja = caja.clone();
            let cancelar = cancelar.clone();
            let estado = estado.clone();
            glib::spawn_future_local(async move {
                yo.importar_fichero(fichero, None, false, &estado).await;
                yo.terminar_fila(&caja, &cancelar);
            });
        });
    }

    /// Importar ficheros o carpetas elegidos por el usuario (o soltados en la ventana).
    pub fn importar_locales(self: &Rc<Self>, rutas: Vec<PathBuf>) {
        for ruta in rutas {
            let nombre = ruta.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (caja, barra, estado, cancelar) = self.fila(&nombre);
            barra.set_fraction(1.0);
            let yo = self.clone();
            glib::spawn_future_local(async move {
                let biblioteca = yo.ctx.biblioteca();
                let sin_audio = yo.ctx.ajustes.boolean("strip-audio");
                let version_30 = yo.ctx.version_30_al_importar();
                estado.set_label("Importando…");
                let r = gio::spawn_blocking(move || {
                    importar::importar(&ruta, &biblioteca, None, sin_audio, false, None, version_30)
                })
                    .await
                    .unwrap_or_else(|_| Err("fallo interno".into()));
                match r {
                    Ok(r) => {
                        estado.set_label(&format!("Listo · {}", r.info.resumen()));
                        yo.ctx.biblioteca_cambiada();
                        yo.ctx.aviso_importado(&r.ruta, &r.avisos);
                    }
                    Err(e) => {
                        estado.set_label(&format!("Error: {e}"));
                        yo.ctx.aviso(&format!("No se pudo importar «{nombre}»: {e}"));
                    }
                }
                yo.terminar_fila(&caja, &cancelar);
            });
        }
    }

    pub fn hay_activas(&self) -> bool {
        self.activas.get() > 0
    }
}
