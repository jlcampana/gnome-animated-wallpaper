// Fondos animados: vídeo en bucle como fondo del escritorio.
//
//  - El vídeo lo reproduce fondos-animados-renderer (Rust + GStreamer, decodificación VA-API),
//    lanzado como cliente Wayland con Meta.WaylandClient, con una ventana sin decoración por
//    monitor titulada «fondos-animados-renderer-<n>».
//  - Esas ventanas se esconden (fuera de Alt+Tab, del dock y de Actividades, y su actor oculto)
//    y su contenido se pinta con un Clutter.Clone dentro de cada Meta.BackgroundActor: el del
//    escritorio y los de las miniaturas de Actividades. Para eso se intercepta
//    BackgroundManager._createBackgroundActor, que crea todos los fondos de GNOME Shell.
//  - El reproductor se pausa con la sesión bloqueada o si todas las pantallas están tapadas por
//    ventanas a pantalla completa (o maximizadas, si se activa).
//  - Órdenes al reproductor por su stdin: «video <ruta>», «fit <modo>», «pause», «play».
import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Background from 'resource:///org/gnome/shell/ui/background.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {QuickMenuToggle, SystemIndicator} from 'resource:///org/gnome/shell/ui/quickSettings.js';
import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';

const TITULO_VENTANA = 'fondos-animados-renderer-';
const NOMBRE_CLON = 'fondos-animados-clon';
const EJECUTABLE = 'fondos-animados-renderer';
const REINTENTOS_MAX = 5;       // arranques fallidos seguidos antes de rendirse
const REINTENTO_MS = 2000;
const ICONO = 'video-x-generic-symbolic';
const ID_APP = 'io.github.jlcampana.FondosAnimados';
const EXTENSIONES_VIDEO = ['.mp4', '.webm', '.mkv', '.mov', '.m4v'];
const MAX_VIDEOS_MENU = 15;

function carpetaBiblioteca(ajustes) {
    const dir = ajustes.get_string('library-dir');
    if (dir)
        return dir;
    const videos = GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_VIDEOS) ??
        GLib.build_filenamev([GLib.get_home_dir(), 'Videos']);
    return GLib.build_filenamev([videos, 'Fondos animados']);
}

// Vídeos de la biblioteca, por nombre. Síncrono: es una carpeta local y pequeña.
function listarVideos(ajustes) {
    const videos = [];
    try {
        const dir = Gio.File.new_for_path(carpetaBiblioteca(ajustes));
        const e = dir.enumerate_children('standard::name,standard::type',
            Gio.FileQueryInfoFlags.NONE, null);
        for (let info; (info = e.next_file(null));) {
            const nombre = info.get_name();
            if (info.get_file_type() === Gio.FileType.REGULAR &&
                EXTENSIONES_VIDEO.some(x => nombre.toLowerCase().endsWith(x)))
                videos.push(dir.get_child(nombre).get_path());
        }
        e.close(null);
    } catch {
        // Sin biblioteca todavía.
    }
    return videos.sort((a, b) => a.localeCompare(b));
}

const nombreVideo = ruta => GLib.path_get_basename(ruta).replace(/\.[^.]+$/, '');

// Esquinas redondeadas para el clon de las miniaturas de Actividades: el fondo estático las
// recorta con rounded_clip_radius de Meta.BackgroundContent, que no afecta a los hijos.
const DECLARACIONES = `
uniform vec2 tamano;
uniform vec4 limites;
uniform float radio;
`;
const CODIGO = `
vec2 p = cogl_tex_coord0_in.xy * tamano;
vec2 minimo = limites.xy + vec2(radio);
vec2 maximo = limites.zw - vec2(radio);
vec2 d = max(max(minimo - p, p - maximo), vec2(0.0));
float alfa = radio > 0.0 ? clamp(radio - length(d) + 0.5, 0.0, 1.0) : 1.0;
cogl_color_out *= alfa;
`;

const EfectoEsquinas = GObject.registerClass({GTypeName: 'FondosAnimadosEfectoEsquinas'},
class EfectoEsquinas extends Shell.GLSLEffect {
    _init(contenido) {
        super._init();
        this._contenido = contenido;
        this._uTamano = this.get_uniform_location('tamano');
        this._uLimites = this.get_uniform_location('limites');
        this._uRadio = this.get_uniform_location('radio');
    }

    vfunc_build_pipeline() {
        this.add_glsl_snippet(Cogl.SnippetHook.FRAGMENT, DECLARACIONES, CODIGO, false);
    }

    vfunc_paint_target(nodo, contexto) {
        const actor = this.get_actor();
        const [ancho, alto] = actor.get_size();
        const limites = this._limites ?? [0, 0, ancho, alto];
        this.set_uniform_float(this._uTamano, 2, [ancho, alto]);
        this.set_uniform_float(this._uLimites, 4, limites);
        this.set_uniform_float(this._uRadio, 1, [this._contenido.rounded_clip_radius]);
        super.vfunc_paint_target(nodo, contexto);
    }

    setLimites(x, y, ancho, alto) {
        this._limites = [x, y, x + ancho, y + alto];
        this.queue_repaint();
    }
});

const BotonFondos = GObject.registerClass(
class BotonFondos extends QuickMenuToggle {
    _init(ext) {
        super._init({title: 'Fondo animado', iconName: ICONO, toggleMode: true});
        this._ext = ext;
        this._ajustes = ext._ajustes;

        this.menu.setHeader(ICONO, 'Fondo animado', 'Vídeo en bucle de fondo de escritorio');

        this._pausa = new PopupMenu.PopupSwitchMenuItem('Pausar', false);
        this._pausa.connect('toggled', (_item, estado) => {
            if (!this._actualizando)
                ext.pausarManual(estado);
        });
        this.menu.addMenuItem(this._pausa);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('Biblioteca'));
        this._lista = new PopupMenu.PopupMenuSection();
        this.menu.addMenuItem(this._lista);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this.menu.addAction('Abrir Fondos animados', () => {
            Main.overview.hide();
            Main.panel.closeQuickSettings();
            Gio.DesktopAppInfo.new(`${ID_APP}.desktop`)?.launch([], null);
        });

        this.connect('clicked', () => {
            this._ajustes.set_boolean('enabled', !this._ajustes.get_boolean('enabled'));
        });
        this.menu.connect('open-state-changed', (_m, abierto) => {
            if (abierto)
                this._rellenarLista();
        });
        this._ajustes.connectObject('changed', () => this._actualizar(), this);
        this._actualizar();
    }

    _rellenarLista() {
        this._lista.removeAll();
        const actual = this._ajustes.get_string('video');
        const videos = listarVideos(this._ajustes);
        if (videos.length === 0) {
            const vacio = new PopupMenu.PopupMenuItem('No hay vídeos todavía', {reactive: false});
            this._lista.addMenuItem(vacio);
            return;
        }
        for (const ruta of videos.slice(0, MAX_VIDEOS_MENU)) {
            const item = new PopupMenu.PopupMenuItem(nombreVideo(ruta));
            item.setOrnament(ruta === actual ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
            item.connect('activate', () => {
                this._ajustes.set_string('video', ruta);
                this._ajustes.set_boolean('enabled', true);
            });
            this._lista.addMenuItem(item);
        }
        if (videos.length > MAX_VIDEOS_MENU) {
            const mas = new PopupMenu.PopupMenuItem(
                `y ${videos.length - MAX_VIDEOS_MENU} más en la app…`, {reactive: false});
            this._lista.addMenuItem(mas);
        }
    }

    _actualizar() {
        this._actualizando = true;
        const activo = this._ajustes.get_boolean('enabled');
        const video = this._ajustes.get_string('video');
        this.checked = activo;
        this._pausa.setToggleState(this._ext.pausaManual);
        this._pausa.visible = activo;
        if (!activo)
            this.subtitle = 'Desactivado';
        else if (!video)
            this.subtitle = 'Sin vídeo';
        else
            this.subtitle = (this._ext.pausaManual ? '⏸ ' : '') + nombreVideo(video);
        if (this.menu.isOpen)
            this._rellenarLista();
        this._actualizando = false;
    }

    destroy() {
        this._ajustes.disconnectObject(this);
        super.destroy();
    }
});

const Indicador = GObject.registerClass(
class Indicador extends SystemIndicator {
    _init(ext) {
        super._init();
        this.boton = new BotonFondos(ext);
        this.quickSettingsItems.push(this.boton);
    }

    destroy() {
        this.quickSettingsItems.forEach(item => item.destroy());
        super.destroy();
    }
});

export default class FondosAnimados extends Extension {
    enable() {
        this._ajustes = this.getSettings();
        this._clones = new Set();      // Clutter.Clone dentro de cada fondo
        this._ventanas = new Map();    // índice de monitor -> Meta.Window del reproductor
        this._cliente = null;          // Meta.WaylandClient
        this._proceso = null;          // Gio.Subprocess
        this._entrada = null;          // stdin del reproductor
        this._fallos = 0;
        this._pausado = false;
        this._idPausa = 0;
        this._idReintento = 0;
        this.pausaManual = false;

        this._inyecciones = new InjectionManager();
        const yo = this;
        this._inyecciones.overrideMethod(Background.BackgroundManager.prototype,
            '_createBackgroundActor', original => function (...args) {
                const actor = original.call(this, ...args);
                yo._decorarFondo(actor, this._monitorIndex);
                return actor;
            });
        // Los fondos que ya existen: los del escritorio se rehacen; los de Actividades se
        // buscan en el árbol de actores.
        Main.layoutManager._updateBackgrounds();
        this._decorarExistentes(Main.uiGroup);

        global.display.connectObject('window-created', (_d, w) => this._ventanaCreada(w), this);
        global.display.connectObject('notify::focus-window', () => this._noEnfocar(), this);
        Main.layoutManager.connectObject('monitors-changed', () => this._reiniciar(), this);

        this._ajustes.connectObject(
            'changed::enabled', () => this._reiniciar(),
            'changed::renderer-path', () => this._reiniciar(),
            'changed::debug', () => this._reiniciar(),
            'changed::video', () => this._cambiarVideo(),
            'changed::fit', () => this._enviar(`fit ${this._ajustes.get_string('fit')}`),
            'changed::pause-fullscreen', () => this._colaPausa(),
            'changed::pause-maximized', () => this._colaPausa(),
            'changed::pause-locked', () => this._colaPausa(),
            this);

        const cola = () => this._colaPausa();
        global.display.connectObject('restacked', cola, 'in-fullscreen-changed', cola, this);
        global.window_manager.connectObject('size-changed', cola, 'minimize', cola,
            'unminimize', cola, 'map', cola, 'destroy', cola, this);
        global.workspace_manager.connectObject('active-workspace-changed', cola, this);
        Main.overview.connectObject('showing', cola, 'hidden', cola, this);
        Main.sessionMode.connectObject('updated', () => {
            if (this._indicador)
                this._indicador.boton.visible = !Main.sessionMode.isLocked;
            cola();
        }, this);

        this._arrancar();

        // En la pantalla de bloqueo no hay Ajustes rápidos: el botón solo en sesión normal.
        if (Main.sessionMode.currentMode === 'user' || Main.sessionMode.parentMode === 'user') {
            this._indicador = new Indicador(this);
            Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicador);
        }
    }

    disable() {
        this._indicador?.destroy();
        this._indicador = null;
        this._parar();
        if (this._idPausa)
            GLib.source_remove(this._idPausa);
        if (this._idReintento)
            GLib.source_remove(this._idReintento);
        this._idPausa = this._idReintento = 0;

        global.display.disconnectObject(this);
        global.window_manager.disconnectObject(this);
        global.workspace_manager.disconnectObject(this);
        Main.layoutManager.disconnectObject(this);
        Main.overview.disconnectObject(this);
        Main.sessionMode.disconnectObject(this);
        this._ajustes.disconnectObject(this);

        this._inyecciones.clear();
        this._inyecciones = null;
        for (const clon of [...this._clones])
            clon.destroy();
        this._clones = null;
        Main.layoutManager._updateBackgrounds();
        this._ajustes = null;
    }

    // ── Fondos ──────────────────────────────────────────────────────────────

    _decorarExistentes(actor) {
        if (actor instanceof Meta.BackgroundActor) {
            if (!actor.get_children().some(c => c.name === NOMBRE_CLON))
                this._decorarFondo(actor, actor.monitor);
            return;
        }
        for (const hijo of actor.get_children())
            this._decorarExistentes(hijo);
    }

    _decorarFondo(fondo, monitor) {
        const clon = new Clutter.Clone({name: NOMBRE_CLON, reactive: false});
        clon.add_constraint(new Clutter.BindConstraint({
            source: fondo, coordinate: Clutter.BindCoordinate.SIZE,
        }));
        clon._monitor = monitor;

        // Fondos de Actividades (no son hijos del grupo de fondos del escritorio): esquinas
        // redondeadas como las del fondo estático, dentro del área de trabajo.
        if (fondo.get_parent() !== Main.layoutManager._backgroundGroup) {
            const efecto = new EfectoEsquinas(fondo.content);
            clon.add_effect(efecto);
            const actualizar = () => {
                const m = Main.layoutManager.monitors[monitor];
                if (!m)
                    return;
                const area = Main.layoutManager.getWorkAreaForMonitor(monitor);
                // El contenido estira el monitor entero sobre el actor, con escala distinta
                // en cada eje; los límites del recorte van en coordenadas del monitor.
                const [ancho, alto] = fondo.get_size();
                const ex = ancho / Math.max(m.width, 1);
                const ey = alto / Math.max(m.height, 1);
                efecto.setLimites((area.x - m.x) * ex, (area.y - m.y) * ey,
                    area.width * ex, area.height * ey);
            };
            fondo.content.connectObject('notify::rounded-clip-radius', () => clon.queue_redraw(), clon);
            fondo.connectObject('notify::size', actualizar, clon);
            global.display.connectObject('workareas-changed', actualizar, clon);
            actualizar();
        }

        fondo.add_child(clon);
        this._clones.add(clon);
        clon.connect('destroy', () => this._clones?.delete(clon));
        this._enlazarClon(clon);
    }

    _enlazarClon(clon) {
        const ventana = this._ventanas.get(clon._monitor) ?? this._ventanas.get(0);
        const actor = ventana?.get_compositor_private() ?? null;
        clon.source = actor;
        clon.visible = actor !== null;
    }

    _enlazarTodos() {
        for (const clon of this._clones)
            this._enlazarClon(clon);
    }

    // ── Reproductor ─────────────────────────────────────────────────────────

    _buscarEjecutable() {
        const configurado = this._ajustes.get_string('renderer-path');
        if (configurado)
            return configurado;
        for (const dir of [GLib.build_filenamev([GLib.get_home_dir(), '.local', 'bin']),
            GLib.build_filenamev([GLib.get_home_dir(), 'scripts'])]) {
            const ruta = GLib.build_filenamev([dir, EJECUTABLE]);
            if (GLib.file_test(ruta, GLib.FileTest.IS_EXECUTABLE))
                return ruta;
        }
        return GLib.find_program_in_path(EJECUTABLE);
    }

    _arrancar() {
        if (this._proceso || !this._ajustes.get_boolean('enabled'))
            return;
        const video = this._ajustes.get_string('video');
        if (!video || !GLib.file_test(video, GLib.FileTest.EXISTS))
            return;
        const ejecutable = this._buscarEjecutable();
        if (!ejecutable) {
            console.warn('Fondos animados: no se encuentra fondos-animados-renderer');
            return;
        }

        const monitores = Main.layoutManager.monitors
            .map(m => `${m.width}x${m.height}`).join(',');
        const argv = [ejecutable, '--monitors', monitores, '--video', video,
            '--fit', this._ajustes.get_string('fit')];
        if (this._ajustes.get_boolean('debug'))
            argv.push('--debug');
        this._depurar(`lanzo ${argv.join(' ')}`);

        const lanzador = new Gio.SubprocessLauncher({
            flags: Gio.SubprocessFlags.STDIN_PIPE | Gio.SubprocessFlags.STDOUT_PIPE |
                Gio.SubprocessFlags.STDERR_MERGE,
        });
        try {
            this._cliente = Meta.WaylandClient.new_subprocess(global.context, lanzador, argv);
            this._proceso = this._cliente.get_subprocess();
        } catch (e) {
            console.error(`Fondos animados: no se pudo lanzar el reproductor: ${e.message}`);
            this._cliente = this._proceso = null;
            return;
        }
        lanzador.close();

        this._entrada = this._proceso.get_stdin_pipe();
        this._leerSalida(Gio.DataInputStream.new(this._proceso.get_stdout_pipe()));
        const proceso = this._proceso;
        const inicio = GLib.get_monotonic_time();
        proceso.wait_async(null, () => {
            if (this._proceso !== proceso)
                return;
            this._limpiarProceso();
            // Si muere al poco de arrancar, se reintenta unas pocas veces.
            const vivioMs = (GLib.get_monotonic_time() - inicio) / 1000;
            this._fallos = vivioMs < 10000 ? this._fallos + 1 : 0;
            if (this._fallos >= REINTENTOS_MAX) {
                console.error('Fondos animados: el reproductor falla al arrancar; me rindo');
                return;
            }
            this._idReintento = GLib.timeout_add(GLib.PRIORITY_DEFAULT, REINTENTO_MS, () => {
                this._idReintento = 0;
                this._arrancar();
                return GLib.SOURCE_REMOVE;
            });
        });
        this._pausado = false;
        this._colaPausa();
    }

    _leerSalida(flujo) {
        flujo.read_line_async(GLib.PRIORITY_LOW, null, (f, res) => {
            try {
                const [linea] = f.read_line_finish_utf8(res);
                if (linea === null)
                    return;
                console.log(`Fondos animados (reproductor): ${linea}`);
                this._leerSalida(f);
            } catch {
                // Flujo cerrado: el proceso ha terminado.
            }
        });
    }

    _limpiarProceso() {
        this._proceso = null;
        this._cliente = null;
        this._entrada = null;
        this._ventanas.clear();
        if (this._clones)
            this._enlazarTodos();
    }

    _parar() {
        const proceso = this._proceso;
        if (!proceso)
            return;
        this._enviar('quit');
        this._limpiarProceso();
        // Si no se va por las buenas, a la fuerza.
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
            proceso.force_exit();
            return GLib.SOURCE_REMOVE;
        });
    }

    _reiniciar() {
        this._fallos = 0;
        this._parar();
        this._arrancar();
    }

    _cambiarVideo() {
        const video = this._ajustes.get_string('video');
        if (this._proceso && video)
            this._enviar(`video ${video}`);
        else
            this._reiniciar();
    }

    // Solo con la clave «debug»: journalctl --user -b | grep 'Fondos animados'
    _depurar(texto) {
        if (this._ajustes?.get_boolean('debug'))
            console.log(`Fondos animados [depuración]: ${texto}`);
    }

    _enviar(orden) {
        this._depurar(`orden → reproductor: ${orden}`);
        if (!this._entrada)
            return;
        try {
            this._entrada.write_all(new TextEncoder().encode(`${orden}\n`), null);
        } catch (e) {
            console.warn(`Fondos animados: no se pudo enviar «${orden}»: ${e.message}`);
        }
    }

    _esNuestra(ventana) {
        if (!this._cliente)
            return false;
        return this._cliente.owns_window?.(ventana) ??
            this._cliente.query_window_belongs_to?.(ventana) ?? false;
    }

    _ventanaCreada(ventana) {
        if (!this._esNuestra(ventana))
            return;
        // GNOME 50 lo tiene en Meta.Window; versiones anteriores, en Meta.WaylandClient.
        if (ventana.hide_from_window_list)
            ventana.hide_from_window_list();
        else
            this._cliente.hide_from_window_list?.(ventana);
        ventana.stick();
        // Tipo escritorio: siempre al fondo de la pila y fuera de las listas de ventanas.
        ventana.set_type?.(Meta.WindowType.DESKTOP);

        const prepararActor = () => {
            const actor = ventana.get_compositor_private();
            if (!actor)
                return false;
            // El actor real no se ve (ni recibe clics): solo se ven sus clones.
            actor.hide();
            actor.connectObject('notify::visible', () => {
                if (actor.visible)
                    actor.hide();
            }, this);
            return true;
        };

        const titulo = () => {
            const t = ventana.get_title() ?? '';
            if (!t.startsWith(TITULO_VENTANA))
                return;
            const indice = parseInt(t.slice(TITULO_VENTANA.length));
            if (Number.isNaN(indice) || this._ventanas.get(indice) === ventana)
                return;
            this._ventanas.set(indice, ventana);
            this._depurar(`ventana del reproductor ${indice} (monitor ${indice}), ${this._clones.size} fondos clonados`);
            const m = Main.layoutManager.monitors[indice];
            if (m)
                ventana.move_frame(false, m.x, m.y);
            if (!prepararActor())
                ventana.connectObject('shown', () => prepararActor(), this);
            this._enlazarTodos();
        };
        ventana.connectObject('notify::title', titulo, this);
        ventana.connectObject('unmanaged', () => {
            ventana.disconnectObject(this);
            for (const [i, w] of this._ventanas) {
                if (w === ventana)
                    this._ventanas.delete(i);
            }
            this._enlazarTodos();
        }, this);
        titulo();
    }

    // Las ventanas del reproductor no deben quedarse con el foco del teclado.
    _noEnfocar() {
        const foco = global.display.focus_window;
        if (foco && this._esNuestra(foco))
            global.display.unset_input_focus?.(global.get_current_time());
    }

    // ── Pausa ───────────────────────────────────────────────────────────────

    _colaPausa() {
        if (this._idPausa)
            return;
        this._idPausa = GLib.idle_add(GLib.PRIORITY_LOW, () => {
            this._idPausa = 0;
            this._actualizarPausa();
            return GLib.SOURCE_REMOVE;
        });
    }

    pausarManual(pausa) {
        this.pausaManual = pausa;
        this._indicador?.boton._actualizar();
        this._colaPausa();
    }

    _debePausar() {
        if (this.pausaManual)
            return true;
        if (Main.sessionMode.isLocked)
            return this._ajustes.get_boolean('pause-locked');
        if (Main.overview.visible)
            return false;
        const pantallaCompleta = this._ajustes.get_boolean('pause-fullscreen');
        const maximizada = this._ajustes.get_boolean('pause-maximized');
        if (!pantallaCompleta && !maximizada)
            return false;

        const tapados = new Set();
        const espacio = global.workspace_manager.get_active_workspace();
        for (const w of espacio.list_windows()) {
            if (this._esNuestra(w) || w.minimized || !w.showing_on_its_workspace())
                continue;
            if (w.get_window_type() !== Meta.WindowType.NORMAL)
                continue;
            if ((pantallaCompleta && w.is_fullscreen()) || (maximizada && w.is_maximized()))
                tapados.add(w.get_monitor());
        }
        return tapados.size >= Main.layoutManager.monitors.length;
    }

    _actualizarPausa() {
        const pausar = this._debePausar();
        if (pausar === this._pausado || !this._proceso)
            return;
        this._depurar(pausar ? 'pausa (pantallas tapadas, bloqueo o pausa manual)' : 'reanudo');
        this._pausado = pausar;
        this._enviar(pausar ? 'pause' : 'play');
    }
}
