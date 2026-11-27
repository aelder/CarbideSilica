Carbide Silica for Linux

From the extracted folder, run:

    ./install.sh

This installs the explorer, demo launcher and desktop shortcut under ~/.local.
Run the demo from a graphical Linux session:

    ~/.local/bin/carbide-silica-demo

To open the tip block directly, add --open 5000. The dashboard and explorer use
a temporary synthetic chain. Closing the window stops the local RPC server and
clears the demo cache. You can also choose Run Synthetic Demo from the Carbide
Silica desktop shortcut's actions.

To use your Verus node, launch Carbide Silica from the application menu or run
~/.local/bin/carbide-silica.

The binaries use system GTK 4.14 or newer, ALSA, fontconfig and CA certificates.
The demo uses Python 3. To remove the installed files, run ./uninstall.sh from
this folder. If you supplied a custom prefix to install.sh, pass the same prefix
to uninstall.sh.
