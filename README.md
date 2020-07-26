# voicesurf

Allows clicking links in Firefox by voice. As pre-alpha code, this has a
lot of bugs.

## Installation

Installation is entirely manual right now, but binaries and extensions
will be available in the future. For now:

1. Ensure you have macOS or Linux installed.
   Ensure you have Firefox 79 installed. __Note at the moment this is
   only available on the Beta channel, but will be released in a week.__
2. Compile the native integration with `cd native && cargo build --release`.
3. Create `XDG_RUNTIME_DIR`. By default `~/.run` is hardcoded, you can
   run `mkdir ~/.run && chmod 700 ~/.run` to get that default working.
4. Copy contents of the `talon` directory to your Talon user directory.
5. Edit the path in `native/manifest.json` to be correct, and point to
   `native/exe`.
6. Follow the [MDN documentation](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_manifests#Manifest_location)
   to add `native/manifest.json` as a new manifest.
7. Add the extension in `extension/` by following the instructions
   [here](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Your_first_WebExtension#Installing).

## Usage

Say `surf <X>` to click on the link that contains `<X>`. TF-IDF search
is used, so you only need to be approximately correct and can use
substrings.

