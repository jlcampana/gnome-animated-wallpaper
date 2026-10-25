//! Vista previa de un vídeo en bucle y sin sonido (Explorar), con un playbin3 propio en vez de
//! gtk::Video: con el GtkMediaFile de GTK la app se caía al cerrar diálogos con la vista previa
//! en marcha (su GstPlay se liberaba mientras su hilo aún mandaba cambios de estado).

use std::path::Path;

use gst::prelude::*;
use gtk::{gdk, glib};

pub struct Previa {
    playbin: gst::Element,
    _vigilancia: gst::bus::BusWatchGuard,
}

impl Previa {
    /// Empieza a reproducir `ruta` en `imagen`. None si falta GStreamer o gtk4paintablesink.
    pub fn new(ruta: &Path, imagen: &gtk::Picture) -> Option<Previa> {
        gst::init().ok()?;
        let sink = gst::ElementFactory::make("gtk4paintablesink").build().ok()?;
        imagen.set_paintable(Some(&sink.property::<gdk::Paintable>("paintable")));
        let playbin = gst::ElementFactory::make("playbin3").property("video-sink", &sink).build().ok()?;
        // Solo vídeo: así no hay audio que silenciar.
        playbin.set_property_from_str("flags", "video");
        playbin.set_property("uri", glib::filename_to_uri(ruta, None).ok()?.as_str());

        let debil = playbin.downgrade();
        let vigilancia = playbin
            .bus()?
            .add_watch_local(move |_, msg| {
                if let (gst::MessageView::Eos(_), Some(p)) = (msg.view(), debil.upgrade()) {
                    let _ = p.seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, gst::ClockTime::ZERO);
                }
                glib::ControlFlow::Continue
            })
            .ok()?;
        playbin.set_state(gst::State::Playing).ok()?;
        Some(Previa { playbin, _vigilancia: vigilancia })
    }
}

impl Drop for Previa {
    /// Parada síncrona: al volver, los hilos de GStreamer ya han terminado.
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
    }
}
