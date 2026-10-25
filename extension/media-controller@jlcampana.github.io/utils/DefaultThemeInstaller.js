import GLib from "gi://GLib";
import Gio from "gi://Gio";
import Soup from "gi://Soup";
import { gettext as _ } from "resource:///org/gnome/shell/extensions/extension.js";

import {
  fetchBytes,
  extractArchive,
  scanForThemeFolderRecursive,
  copyDirRecursive,
  rimraf,
} from "./core/themeUtils.js";
import { USER_THEMES_DIR, OLD_USER_THEMES_DIR } from "../ui/themes/constants.js";

const MATRIX_ARCHIVE_URL =
  "https://codeberg.org/sanjai-shaarugesh/matrix/archive/main.zip";

const THEMES_DIR = USER_THEMES_DIR;
const SENTINEL = GLib.build_filenamev([
  THEMES_DIR,
  ".matrix-default-installed",
]);

async function installMatrixTheme(cancellable) {
  GLib.mkdir_with_parents(THEMES_DIR, 0o755);

  const tmpBase = GLib.build_filenamev([
    GLib.get_tmp_dir(),
    `amc-matrix-${GLib.get_monotonic_time()}`,
  ]);
  const archivePath = `${tmpBase}.zip`;
  const extractDir = `${tmpBase}-extracted`;
  const session = new Soup.Session({ timeout: 30 });
  const cancelId = cancellable.connect(() => session.abort());

  try {
    GLib.file_set_contents(
      archivePath,
      await fetchBytes(session, MATRIX_ARCHIVE_URL, cancellable),
    );
    await extractArchive(archivePath, extractDir, false);

    const themeFolder = scanForThemeFolderRecursive(extractDir, 4);
    if (!themeFolder) throw new Error(_("No theme folder found in archive"));

    copyDirRecursive(
      themeFolder,
      GLib.build_filenamev([THEMES_DIR, GLib.path_get_basename(themeFolder)]),
    );
    GLib.file_set_contents(SENTINEL, "1");
  } finally {
    cancellable.disconnect(cancelId);
    session.abort();
    rimraf(archivePath);
    rimraf(extractDir);
  }
}

// Moves the themes out of gnome-shell/extensions, where the shell tries to load them as an
// extension ("Missing metadata.json").
function migrateOldThemesDir() {
  if (!GLib.file_test(OLD_USER_THEMES_DIR, GLib.FileTest.IS_DIR)) return;
  if (GLib.file_test(USER_THEMES_DIR, GLib.FileTest.EXISTS)) return;
  try {
    GLib.mkdir_with_parents(GLib.path_get_dirname(USER_THEMES_DIR), 0o755);
    Gio.File.new_for_path(OLD_USER_THEMES_DIR).move(
      Gio.File.new_for_path(USER_THEMES_DIR),
      Gio.FileCopyFlags.NONE,
      null,
      null,
    );
  } catch (e) {
    console.error(`Could not move themes to ${USER_THEMES_DIR}: ${e.message}`);
  }
}

// Returns a Gio.Cancellable, cancel it from disable()
export function installDefaultThemesIfNeeded() {
  const cancellable = new Gio.Cancellable();

  migrateOldThemesDir();

  if (!GLib.file_test(SENTINEL, GLib.FileTest.EXISTS)) {
    installMatrixTheme(cancellable).catch((e) => {
      if (!e.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED))
        console.error(`Default theme install failed: ${e.message}`);
    });
  }
  return cancellable;
}