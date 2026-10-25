// Preferencias: lo básico aquí; la biblioteca, la conversión y las descargas están en la app
// Fondos animados (io.github.jlcampana.FondosAnimados).
import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

const ID_APP = 'io.github.jlcampana.FondosAnimados';

export default class Preferencias extends ExtensionPreferences {
    fillPreferencesWindow(ventana) {
        const ajustes = this.getSettings();
        const pagina = new Adw.PreferencesPage();
        ventana.add(pagina);

        const app = new Adw.PreferencesGroup();
        const fila = new Adw.ActionRow({
            title: 'Abrir Fondos animados',
            subtitle: 'Biblioteca de vídeos, conversión y descargas',
            activatable: true,
        });
        fila.add_suffix(new Gtk.Image({icon_name: 'go-next-symbolic'}));
        fila.connect('activated', () => {
            const info = Gio.DesktopAppInfo.new(`${ID_APP}.desktop`);
            info?.launch([], null);
        });
        app.add(fila);
        pagina.add(app);

        const grupo = new Adw.PreferencesGroup({title: 'Reproducción'});
        const interruptor = (clave, titulo, subtitulo) => {
            const f = new Adw.SwitchRow({title: titulo, subtitle: subtitulo ?? ''});
            ajustes.bind(clave, f, 'active', Gio.SettingsBindFlags.DEFAULT);
            grupo.add(f);
        };
        interruptor('enabled', 'Fondo animado activo');
        interruptor('pause-fullscreen', 'Pausar con pantalla completa',
            'Cuando todas las pantallas tienen una ventana a pantalla completa');
        interruptor('pause-maximized', 'Pausar con ventanas maximizadas');
        interruptor('pause-locked', 'Pausar con la sesión bloqueada');

        const modos = ['cover', 'contain', 'fill'];
        const ajuste = new Adw.ComboRow({
            title: 'Ajuste a la pantalla',
            model: Gtk.StringList.new(['Recortar para llenar', 'Entero con bandas', 'Estirar']),
        });
        ajuste.selected = Math.max(0, modos.indexOf(ajustes.get_string('fit')));
        ajuste.connect('notify::selected', () => ajustes.set_string('fit', modos[ajuste.selected]));
        grupo.add(ajuste);
        pagina.add(grupo);
    }
}
