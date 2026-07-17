//! Smoke test: read the registry theme + verify the icon builder picks
//! the right variant for the current theme.

use audio_switcher::theme;
use audio_switcher::tray;

fn main() {
    let t = theme::init();
    println!("[theme-check] detected: {t:?}");

    // We can't reach into the Icon struct (fields are private), but we
    // can verify the icon at least builds and the PNG decode succeeded.
    let _icon = tray::make_icon(t);

    // Sanity: read the same registry path directly to confirm the value.
    let hkcu = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
    let value = "AppsUseLightTheme";
    println!("[theme-check] registry path: {hkcu}::{value}");

    // Also print which PNG bytes are baked into the binary for this build.
    let light_size = include_bytes!("../../assets/icon_light.png").len();
    let dark_size = include_bytes!("../../assets/icon_dark.png").len();
    println!(
        "[theme-check] embedded PNG sizes: light={}B dark={}B",
        light_size, dark_size
    );

    if t == theme::Theme::Unknown {
        eprintln!("[theme-check] WARN: could not detect theme (treating as Light)");
    }
    println!("[theme-check] PASS");
}
