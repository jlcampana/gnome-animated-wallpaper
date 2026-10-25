//! Diálogo «Convertir»: resolución, ajuste, fps, tramo, fundido del bucle, códec y calidad.
//! La conversión va a la cola de descargas y conversiones (la hace el gestor, con
//! fondo-animado-convertir).

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::cola::{Conversion, TipoConversion};
use crate::contexto::Contexto;
use crate::descargas::Descargas;
use crate::{config, importar};

const AJUSTES: &[(&str, &str)] = &[
    ("cubrir", "Recortar para llenar"),
    ("encajar", "Entero, con bandas negras"),
    ("estirar", "Estirar"),
];
const FPS: &[&str] = &["original", "24", "30", "60"];
const CODECS: &[(&str, &str)] = &[
    ("auto", "Recomendado para este equipo"),
    ("hevc", "HEVC (H.265)"),
    ("h264", "H.264"),
    ("vp9", "VP9"),
    ("av1", "AV1"),
];

pub fn abrir(ctx: Rc<Contexto>, descargas: Rc<Descargas>, origen: &Path) {
    let (ancho, alto) = config::resolucion_pantalla();
    let resoluciones: Vec<String> = std::iter::once(format!("{ancho}x{alto}"))
        .chain(["1920x1080", "2560x1080", "2560x1440", "3440x1440", "3840x2160"].iter().map(|s| s.to_string()))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut opciones_res = vec![format!("Pantalla ({ancho}×{alto})")];
    opciones_res.extend(resoluciones.iter().map(|r| r.replace('x', "×")));
    opciones_res.push("Original".into());

    let fila_info = adw::ActionRow::builder().title("Original").subtitle("Leyendo…").build();
    {
        let origen = origen.to_path_buf();
        let fila = fila_info.downgrade();
        glib::spawn_future_local(async move {
            let info = gio::spawn_blocking(move || importar::info(&origen)).await;
            if let (Ok(Ok(info)), Some(f)) = (info, fila.upgrade()) {
                f.set_subtitle(&info.resumen());
            }
        });
    }

    let lista = |v: Vec<String>| gtk::StringList::new(&v.iter().map(String::as_str).collect::<Vec<_>>());
    let resolucion = adw::ComboRow::builder().title("Resolución").model(&lista(opciones_res)).build();
    let ajuste = adw::ComboRow::builder()
        .title("Ajuste")
        .model(&lista(AJUSTES.iter().map(|a| a.1.to_string()).collect()))
        .build();
    let fps = adw::ComboRow::builder()
        .title("Fotogramas por segundo")
        .model(&lista(FPS.iter().map(|f| if *f == "original" { "Como el original".into() } else { f.to_string() }).collect()))
        .build();
    let inicio = adw::SpinRow::with_range(0.0, 36000.0, 0.5);
    inicio.set_title("Empezar en (s)");
    inicio.set_digits(1);
    let duracion = adw::SpinRow::with_range(0.0, 36000.0, 0.5);
    duracion.set_title("Duración (s)");
    duracion.set_subtitle("0 = hasta el final");
    duracion.set_digits(1);
    let fundido = adw::SpinRow::with_range(0.0, 5.0, 0.25);
    fundido.set_title("Fundido del bucle (s)");
    fundido.set_subtitle("Funde el final con el principio para que no se note el salto");
    fundido.set_digits(2);
    let codec = adw::ComboRow::builder()
        .title("Formato")
        .model(&lista(CODECS.iter().map(|c| c.1.to_string()).collect()))
        .build();
    {
        let codec = codec.downgrade();
        ctx.conectar_capacidades(move |cap| {
            if let Some(c) = codec.upgrade() {
                let como = if cap.codifica_por_gpu() { "con la GPU" } else { "con la CPU" };
                c.set_subtitle(&format!("Recomendado: {} {como} ({})", cap.recomendado.to_uppercase(), cap.codificador));
            }
        });
    }
    let cpu = adw::SwitchRow::builder()
        .title("Forzar la CPU")
        .subtitle("Más lento, a veces algo más de calidad")
        .build();
    let calidad = adw::SpinRow::with_range(14.0, 40.0, 1.0);
    calidad.set_title("Calidad");
    calidad.set_subtitle("Menos es mejor calidad y más peso");
    calidad.set_value(24.0);

    let base = origen.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let nombre = adw::EntryRow::builder().title("Nombre").text(format!("{base}-{ancho}x{alto}")).build();
    let reemplazar = adw::SwitchRow::builder()
        .title("Sustituir el original")
        .subtitle("Si no, se guarda como un vídeo nuevo")
        .build();
    let usar = adw::SwitchRow::builder().title("Usar de fondo al terminar").active(true).build();

    let grupo_origen = adw::PreferencesGroup::new();
    grupo_origen.add(&fila_info);
    let grupo_imagen = adw::PreferencesGroup::builder().title("Imagen").build();
    for f in [resolucion.upcast_ref::<gtk::Widget>(), ajuste.upcast_ref(), fps.upcast_ref()] {
        grupo_imagen.add(f);
    }
    let grupo_tramo = adw::PreferencesGroup::builder().title("Tramo y bucle").build();
    for f in [inicio.upcast_ref::<gtk::Widget>(), duracion.upcast_ref(), fundido.upcast_ref()] {
        grupo_tramo.add(f);
    }
    let grupo_codec = adw::PreferencesGroup::builder().title("Codificación").build();
    for f in [codec.upcast_ref::<gtk::Widget>(), cpu.upcast_ref(), calidad.upcast_ref()] {
        grupo_codec.add(f);
    }
    let grupo_salida = adw::PreferencesGroup::builder().title("Resultado").build();
    for f in [nombre.upcast_ref::<gtk::Widget>(), reemplazar.upcast_ref(), usar.upcast_ref()] {
        grupo_salida.add(f);
    }

    let pagina = adw::PreferencesPage::new();
    for g in [&grupo_origen, &grupo_imagen, &grupo_tramo, &grupo_codec, &grupo_salida] {
        pagina.add(g);
    }

    let boton = gtk::Button::builder().label("Convertir").css_classes(["suggested-action"]).build();
    let cabecera = adw::HeaderBar::new();
    cabecera.pack_end(&boton);

    let vista = adw::ToolbarView::new();
    vista.add_top_bar(&cabecera);
    vista.set_content(Some(&pagina));

    let dialogo = adw::Dialog::builder()
        .title(format!("Convertir «{base}»"))
        .content_width(560)
        .content_height(720)
        .child(&vista)
        .build();

    let ventana = ctx.ventana.clone();
    let origen = origen.to_path_buf();
    let d = dialogo.downgrade();
    boton.connect_clicked(move |_| {
        let res = match resolucion.selected() as usize {
            0 => format!("{ancho}x{alto}"),
            i if i <= resoluciones.len() => resoluciones[i - 1].clone(),
            _ => "original".into(),
        };
        let reemplazar = reemplazar.is_active();
        let destino = if reemplazar {
            origen.with_extension("mp4")
        } else {
            let biblioteca = ctx.biblioteca();
            let _ = std::fs::create_dir_all(&biblioteca);
            config::ruta_libre(&biblioteca, &config::nombre_limpio(&nombre.text()), "mp4")
        };
        let mut opciones: Vec<String> = vec![
            "-r".into(),
            res,
            "-a".into(),
            AJUSTES[ajuste.selected() as usize].0.into(),
            "-f".into(),
            FPS[fps.selected() as usize].into(),
            "-i".into(),
            inicio.value().to_string(),
            "-d".into(),
            duracion.value().to_string(),
            "-b".into(),
            fundido.value().to_string(),
            "-c".into(),
            CODECS[codec.selected() as usize].0.into(),
            "-q".into(),
            (calidad.value() as i32).to_string(),
        ];
        if cpu.is_active() {
            opciones.push("--cpu".into());
        }
        let conversion = Conversion { tipo: TipoConversion::Personalizada, opciones, reemplazar };
        descargas.convertir(&origen, &destino, conversion, usar.is_active());
        if let Some(d) = d.upgrade() {
            d.close();
        }
    });

    dialogo.present(Some(&ventana));
}
