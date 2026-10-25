# Media Controller

GNOME Shell extension (45–51) with media controls in the top bar for any
MPRIS player: Spotify, VLC, Rhythmbox, browsers…

Fork of [Advanced Media Controller](https://codeberg.org/sanjai-shaarugesh/Advanced-media-controller)
by sanjai-shaarugesh (v6.6), GPL-3.0. Until October 2026 it lived inside
[gnome-animated-wallpaper](https://github.com/jlcampana/gnome-animated-wallpaper).

## What the fork adds

- Immersive popup: the cover fills the background, with a glossy finish, and
  the controls sit on top of it.
- HD covers: browsers only pass a ~120px thumbnail, so the extension looks up
  the high-resolution artwork in the iTunes catalogue. The search tolerates
  YouTube-style titles (`Artist - 'Song' M/V`, `이효리 (Lee Hyori) - …`,
  `LiSA 『紅蓮華』`). You can turn it off in *Popup → Album Art*.
- Optional synced lyrics (off by default) in a small floating panel under the
  popup. Click the cover to hide or show them.
- The popup stays open when you skip tracks in a browser (they report
  `Stopped` for a moment between songs).
- Uses the shell's own font, same as the rest of the top bar.

## Installation

```bash
UUID=media-controller@jlcampana.github.io
DEST=~/.local/share/gnome-shell/extensions/$UUID
rm -rf "$DEST" && mkdir -p "$DEST"
cp -r ./* "$DEST/"
glib-compile-schemas "$DEST/schemas"
gnome-extensions enable $UUID   # after logging out and back in on Wayland
```
