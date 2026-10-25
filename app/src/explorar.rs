//! Página «Explorar»: fuente (web), búsqueda, categoría y resolución; resultados por páginas
//! (botones anterior/siguiente o el número de la página a la que saltar: cada página sustituye a
//! la anterior, así solo hay en memoria las miniaturas de una). Clic en un resultado: detalle con vista previa y descarga.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::contexto::{self, Contexto};
use crate::descargas::Descargas;
use crate::fuentes::{self, Fuente, Resultado};
use crate::previa::Previa;
use crate::{config, red};

pub struct Explorar {
    ctx: Rc<Contexto>,
    descargas: Rc<Descargas>,
    pub raiz: gtk::Box,
    fuente: gtk::DropDown,
    busqueda: gtk::SearchEntry,
    categoria: gtk::DropDown,
    resolucion: gtk::DropDown,
    rejilla: gtk::FlowBox,
    desplazable: gtk::ScrolledWindow,
    estado: gtk::Stack,
    pagina_vacia: adw::StatusPage,
    pagina_error: adw::StatusPage,
    anterior: gtk::Button,
    siguiente: gtk::Button,
    numero_pagina: gtk::SpinButton,
    pagina: Cell<u32>,
    peticion: Cell<u64>,
    resultados: RefCell<Vec<Resultado>>,
}

fn fuente_actual(d: &gtk::DropDown) -> Fuente {
    Fuente::TODAS[(d.selected() as usize).min(Fuente::TODAS.len() - 1)]
}

impl Explorar {
    pub fn new(ctx: Rc<Contexto>, descargas: Rc<Descargas>) -> Rc<Self> {
        let nombres: Vec<&str> = Fuente::TODAS.iter().map(|f| f.nombre()).collect();
        let fuente = gtk::DropDown::from_strings(&nombres);
        fuente.set_tooltip_text(Some("Web de la que buscar"));
        let busqueda = gtk::SearchEntry::builder()
            .placeholder_text("Buscar (en inglés: girl, frieren, rain…)")
            .hexpand(true)
            .search_delay(700)
            .build();
        let categoria = gtk::DropDown::from_strings(&[]);
        categoria.set_tooltip_text(Some("Categoría (si no hay búsqueda)"));
        let mut res = vec!["Cualquier resolución"];
        res.extend(fuentes::RESOLUCIONES);
        let resolucion = gtk::DropDown::from_strings(&res);
        resolucion.set_tooltip_text(Some("Resolución"));

        let barra = gtk::Box::builder()
            .spacing(8)
            .margin_top(12)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        barra.append(&fuente);
        barra.append(&busqueda);
        barra.append(&categoria);
        barra.append(&resolucion);

        let rejilla = gtk::FlowBox::builder()
            .valign(gtk::Align::Start)
            .homogeneous(true)
            .min_children_per_line(3)
            .max_children_per_line(8)
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .column_spacing(12)
            .row_spacing(12)
            .margin_top(6)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let desplazable = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&rejilla)
            .build();
        let cargando = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .halign(gtk::Align::Center)
            .spacing(12)
            .build();
        cargando.append(&adw::Spinner::builder().width_request(48).height_request(48).build());
        cargando.append(&gtk::Label::builder().label("Buscando…").css_classes(["dim-label"]).build());
        let pagina_vacia = adw::StatusPage::builder()
            .icon_name("edit-find-symbolic")
            .title("Sin resultados")
            .description("Prueba con otra búsqueda, categoría o resolución.")
            .build();
        let pagina_error = adw::StatusPage::builder()
            .icon_name("network-error-symbolic")
            .title("No se pudo cargar")
            .build();
        let reintentar = gtk::Button::builder()
            .label("Reintentar")
            .halign(gtk::Align::Center)
            .css_classes(["pill"])
            .build();
        pagina_error.set_child(Some(&reintentar));
        let estado = gtk::Stack::builder().vexpand(true).build();
        estado.add_named(&desplazable, Some("resultados"));
        estado.add_named(&cargando, Some("cargando"));
        estado.add_named(&pagina_vacia, Some("vacio"));
        estado.add_named(&pagina_error, Some("error"));

        let anterior = gtk::Button::builder().icon_name("go-previous-symbolic").tooltip_text("Página anterior").build();
        let siguiente = gtk::Button::builder().icon_name("go-next-symbolic").tooltip_text("Página siguiente").build();
        // Las webs no dicen cuántas páginas hay: si se pasa, sale «Sin resultados».
        let numero_pagina = gtk::SpinButton::with_range(1.0, 9999.0, 1.0);
        numero_pagina.set_numeric(true);
        numero_pagina.set_width_chars(4);
        numero_pagina.set_tooltip_text(Some("Página a la que ir (escribe el número y pulsa Intro)"));
        let etiqueta_pagina = gtk::Label::new(Some("Página"));
        let paginador = gtk::Box::builder()
            .spacing(12)
            .halign(gtk::Align::Center)
            .margin_top(6)
            .margin_bottom(10)
            .build();
        paginador.append(&anterior);
        let ir_a = gtk::Box::builder().spacing(6).valign(gtk::Align::Center).build();
        ir_a.append(&etiqueta_pagina);
        ir_a.append(&numero_pagina);
        paginador.append(&ir_a);
        paginador.append(&siguiente);

        let raiz = gtk::Box::new(gtk::Orientation::Vertical, 0);
        raiz.append(&barra);
        raiz.append(&estado);
        raiz.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        raiz.append(&paginador);

        let yo = Rc::new(Self {
            ctx,
            descargas,
            raiz,
            fuente,
            busqueda,
            categoria,
            resolucion,
            rejilla,
            desplazable,
            estado,
            pagina_vacia,
            pagina_error,
            anterior,
            siguiente,
            numero_pagina,
            pagina: Cell::new(1),
            peticion: Cell::new(0),
            resultados: RefCell::default(),
        });
        yo.actualizar_categorias();

        let debil = Rc::downgrade(&yo);
        let con = move |f: &dyn Fn(&Rc<Explorar>)| {
            if let Some(yo) = debil.upgrade() {
                f(&yo);
            }
        };
        {
            let con = con.clone();
            yo.fuente.connect_selected_notify(move |_| con(&|yo| {
                yo.actualizar_categorias();
                yo.buscar(1);
            }));
        }
        {
            let con = con.clone();
            yo.categoria.connect_selected_notify(move |_| con(&|yo| yo.buscar(1)));
        }
        {
            let con = con.clone();
            yo.resolucion.connect_selected_notify(move |_| con(&|yo| yo.buscar(1)));
        }
        {
            let con = con.clone();
            yo.busqueda.connect_search_changed(move |_| con(&|yo| yo.buscar(1)));
        }
        {
            let con = con.clone();
            yo.anterior.connect_clicked(move |_| con(&|yo| yo.buscar(yo.pagina.get().saturating_sub(1).max(1))));
        }
        {
            let con = con.clone();
            yo.siguiente.connect_clicked(move |_| con(&|yo| yo.buscar(yo.pagina.get() + 1)));
        }
        {
            let con = con.clone();
            yo.numero_pagina.connect_value_changed(move |n| {
                let pagina = n.value_as_int().max(1) as u32;
                con(&|yo| {
                    // También salta cuando buscar() pone el número: entonces ya coincide.
                    if pagina != yo.pagina.get() {
                        yo.buscar(pagina);
                    }
                })
            });
        }
        {
            let con = con.clone();
            reintentar.connect_clicked(move |_| con(&|yo| yo.buscar(yo.pagina.get())));
        }
        {
            let debil = Rc::downgrade(&yo);
            yo.rejilla.connect_child_activated(move |_, hijo| {
                let Some(yo) = debil.upgrade() else { return };
                let i = hijo.index() as usize;
                let r = yo.resultados.borrow().get(i).cloned();
                if let Some(r) = r {
                    yo.detalle(r);
                }
            });
        }
        yo
    }

    fn actualizar_categorias(&self) {
        let f = fuente_actual(&self.fuente);
        let nombres: Vec<&str> = f.categorias().iter().map(|c| c.0).collect();
        self.categoria.set_model(Some(&gtk::StringList::new(&nombres)));
        self.categoria.set_selected(0);
        self.resolucion.set_visible(f.filtra_resolucion());
    }

    /// Primera carga, cuando se entra en la página por primera vez.
    pub fn cargar_si_vacia(self: &Rc<Self>) {
        if self.peticion.get() == 0 {
            self.buscar(1);
        }
    }

    pub fn buscar(self: &Rc<Self>, pagina: u32) {
        let fuente = fuente_actual(&self.fuente);
        let consulta = self.busqueda.text().to_string();
        let categoria = self.categoria.selected() as usize;
        let resolucion = match self.resolucion.selected() {
            0 => None,
            i if fuente.filtra_resolucion() => fuentes::RESOLUCIONES.get(i as usize - 1).map(|s| s.to_string()),
            _ => None,
        };

        if !fuente.nativa() {
            // vsthemes: al navegador integrado.
            let url = fuente.url_listado(&consulta, categoria, None, pagina);
            self.ctx.navegar(&url);
            return;
        }

        let n = self.peticion.get() + 1;
        self.peticion.set(n);
        self.pagina.set(pagina);
        self.numero_pagina.set_value(pagina as f64);
        self.anterior.set_sensitive(false);
        self.siguiente.set_sensitive(false);
        self.estado.set_visible_child_name("cargando");
        // Fuera la página anterior (y sus miniaturas) antes de pedir la nueva.
        self.vaciar();

        let yo = self.clone();
        glib::spawn_future_local(async move {
            let r = gio::spawn_blocking(move || fuente.buscar(&consulta, categoria, resolucion.as_deref(), pagina))
                .await
                .unwrap_or_else(|_| Err("fallo interno".into()));
            if yo.peticion.get() != n {
                return; // ya hay otra búsqueda más nueva
            }
            yo.anterior.set_sensitive(pagina > 1);
            match r {
                Ok(resultados) if resultados.is_empty() => {
                    yo.pagina_vacia.set_description(Some(if pagina > 1 {
                        "No hay tantas páginas: prueba con un número más bajo."
                    } else {
                        "Prueba con otra búsqueda, categoría o resolución."
                    }));
                    yo.estado.set_visible_child_name("vacio");
                }
                Ok(resultados) => {
                    yo.siguiente.set_sensitive(true);
                    for r in &resultados {
                        yo.rejilla.append(&tarjeta(r, fuente.web()));
                    }
                    yo.resultados.replace(resultados);
                    yo.desplazable.vadjustment().set_value(0.0);
                    yo.estado.set_visible_child_name("resultados");
                }
                Err(e) => {
                    yo.pagina_error.set_description(Some(&glib::markup_escape_text(&e)));
                    yo.estado.set_visible_child_name("error");
                }
            }
        });
    }

    fn vaciar(&self) {
        while let Some(hijo) = self.rejilla.first_child() {
            self.rejilla.remove(&hijo);
        }
        self.resultados.borrow_mut().clear();
    }

    fn detalle(self: &Rc<Self>, r: Resultado) {
        let fuente = fuente_actual(&self.fuente);
        let video = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).height_request(300).build();
        let reproductor: Rc<RefCell<Option<Previa>>> = Rc::default();
        let cerrado = Rc::new(Cell::new(false));
        let imagen = gtk::Picture::builder().content_fit(gtk::ContentFit::Cover).height_request(300).build();
        contexto::cargar_miniatura_remota(&imagen, &r.miniatura, fuente.web());
        let previa = gtk::Stack::builder().css_classes(["card"]).overflow(gtk::Overflow::Hidden).build();
        previa.add_named(&imagen, Some("imagen"));
        previa.add_named(&video, Some("video"));

        let titulo = gtk::Label::builder()
            .label(&r.titulo)
            .wrap(true)
            .xalign(0.0)
            .css_classes(["title-2"])
            .build();
        let info = gtk::Label::builder()
            .label(format!(
                "{} · {}",
                fuente.nombre(),
                r.resolucion.as_deref().map(|x| x.replace('x', "×")).unwrap_or_else(|| "resolución desconocida".into())
            ))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();

        let opciones = gtk::DropDown::from_strings(&[]);
        opciones.set_visible(false);
        let descargar = gtk::Button::builder().label("Descargar").sensitive(false).build();
        let usar = gtk::Button::builder()
            .label("Descargar y poner de fondo")
            .css_classes(["suggested-action"])
            .sensitive(false)
            .build();
        let web = gtk::Button::builder()
            .icon_name("web-browser-symbolic")
            .tooltip_text("Abrir la página en el navegador integrado")
            .build();
        let botones = gtk::Box::builder().spacing(8).halign(gtk::Align::End).build();
        botones.append(&web);
        botones.append(&opciones);
        botones.append(&descargar);
        botones.append(&usar);

        let caja = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(18)
            .margin_start(18)
            .margin_end(18)
            .build();
        caja.append(&previa);
        caja.append(&titulo);
        caja.append(&info);
        caja.append(&botones);
        let vista = adw::ToolbarView::new();
        vista.add_top_bar(&adw::HeaderBar::new());
        vista.set_content(Some(&caja));
        let dialogo = adw::Dialog::builder()
            .title(&r.titulo)
            .content_width(720)
            .child(&vista)
            .build();
        {
            // Al cerrar, fuera el vídeo de la vista previa.
            let reproductor = reproductor.clone();
            let cerrado = cerrado.clone();
            dialogo.connect_closed(move |_| {
                cerrado.set(true);
                reproductor.take();
            });
        }
        {
            let ctx = self.ctx.clone();
            let pagina = r.pagina.clone();
            let d = dialogo.downgrade();
            web.connect_clicked(move |_| {
                if let Some(d) = d.upgrade() {
                    d.close();
                }
                ctx.navegar(&pagina);
            });
        }

        // Detalle (enlaces de descarga y vista previa) en segundo plano.
        let yo = self.clone();
        let referer = r.pagina.clone();
        let titulo_r = r.titulo.clone();
        let d = dialogo.downgrade();
        glib::spawn_future_local(async move {
            let r2 = r.clone();
            let det = gio::spawn_blocking(move || fuente.detalle(&r2)).await.unwrap_or_else(|_| Err("fallo interno".into()));
            let det = match det {
                Ok(det) => det,
                Err(e) => {
                    yo.ctx.aviso(&format!("No se pudo leer la página: {e}"));
                    return;
                }
            };
            if det.descargas.is_empty() {
                yo.ctx.aviso("Esta página no tiene enlace de descarga");
                return;
            }
            let etiquetas: Vec<&str> = det.descargas.iter().map(|x| x.etiqueta.as_str()).collect();
            opciones.set_model(Some(&gtk::StringList::new(&etiquetas)));
            opciones.set_visible(det.descargas.len() > 1);
            // motionbgs: HD o 4K según el ajuste de calidad.
            if yo.ctx.ajustes.string("download-quality") == "hd" {
                if let Some(i) = det.descargas.iter().position(|x| x.etiqueta.starts_with("HD")) {
                    opciones.set_selected(i as u32);
                }
            }
            let descargas = det.descargas.clone();
            let lanzar = {
                let yo = yo.clone();
                let opciones = opciones.clone();
                let referer = referer.clone();
                let titulo = titulo_r.clone();
                let d = d.clone();
                Rc::new(move |usar: bool| {
                    let i = opciones.selected() as usize;
                    if let Some(x) = descargas.get(i).or(descargas.first()) {
                        yo.descargas.descargar(&titulo, &x.url, &referer, usar);
                    }
                    if let Some(d) = d.upgrade() {
                        d.close();
                    }
                })
            };
            {
                let lanzar = lanzar.clone();
                descargar.connect_clicked(move |_| lanzar(false));
            }
            usar.connect_clicked(move |_| lanzar(true));
            descargar.set_sensitive(true);
            usar.set_sensitive(true);

            // Vista previa: se baja a la caché y se reproduce en bucle, sin sonido.
            if let Some(url) = det.vista_previa {
                let referer = referer.clone();
                let ruta = gio::spawn_blocking(move || {
                    let clave = glib::compute_checksum_for_string(glib::ChecksumType::Md5, &url)?;
                    let ext = url.rsplit('.').next().filter(|e| e.len() <= 4).unwrap_or("mp4").to_string();
                    let ruta = config::carpeta_cache().join("previas").join(format!("{clave}.{ext}"));
                    if !ruta.is_file() {
                        std::fs::create_dir_all(ruta.parent()?).ok()?;
                        red::descargar(&url, Some(&referer), &ruta, &|_, _| {}, &|| false).ok()?;
                    }
                    Some(ruta)
                })
                .await
                .ok()
                .flatten();
                // Si el diálogo se cerró mientras se bajaba, ya no se reproduce.
                if let Some(ruta) = ruta.filter(|_| !cerrado.get()) {
                    if let Some(p) = Previa::new(&ruta, &video) {
                        reproductor.replace(Some(p));
                        previa.set_visible_child_name("video");
                    }
                }
            }
        });

        dialogo.present(Some(&self.ctx.ventana));
    }
}

fn tarjeta(r: &Resultado, web: &str) -> gtk::FlowBoxChild {
    let imagen = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .width_request(240)
        .height_request(135)
        .css_classes(["card"])
        .overflow(gtk::Overflow::Hidden)
        .build();
    contexto::cargar_miniatura_remota(&imagen, &r.miniatura, web);
    let encima = gtk::Overlay::builder().child(&imagen).build();
    if let Some(res) = &r.resolucion {
        let etiqueta = gtk::Label::builder()
            .label(res.replace('x', "×"))
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(6)
            .margin_bottom(6)
            .css_classes(["etiqueta-resolucion", "caption"])
            .build();
        encima.add_overlay(&etiqueta);
    }
    let titulo = gtk::Label::builder()
        .label(&r.titulo)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(24)
        .tooltip_text(&r.titulo)
        .build();
    let caja = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(6).build();
    caja.append(&encima);
    caja.append(&titulo);
    gtk::FlowBoxChild::builder().child(&caja).build()
}
