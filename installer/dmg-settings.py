# The macOS .dmg's window, for dmgbuild (release.yml):
#   dmgbuild -s installer/dmg-settings.py -D app=path/to/Omacus.app Omacus out.dmg
# The app on the left, Applications on the right, and the background's arrow
# between them (installer/art/dmg.svg, rendered by make-art.sh; the .tiff holds
# the 1x and Retina sizes and is made in CI with tiffutil).
import glob
import os.path

app = defines["app"]  # noqa: F821 (dmgbuild provides it)
here = os.path.dirname(os.path.abspath(settings_file))  # noqa: F821

format = "UDZO"
files = [app]
symlinks = {"Applications": "/Applications"}
icon = (glob.glob(os.path.join(app, "Contents", "Resources", "*.icns")) or [None])[0]
background = os.path.join(here, "dmg-background.tiff")

window_rect = ((200, 140), (660, 400))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 128
text_size = 13
icon_locations = {
    "Omacus.app": (165, 205),
    "Applications": (495, 205),
}
