//! Internationalisation helpers.
//!
//! `tr` translates a message key for the given [`Lang`].

use crate::config::Lang;

/// Translate `key` for `lang`.
///
/// Unknown keys are returned verbatim, which keeps menus debuggable.
#[must_use]
pub fn tr(key: &str, lang: Lang) -> String {
    let zh = lang.is_zh();
    match key {
        "mute" => {
            if zh {
                "全局静音".into()
            } else {
                "Mute".into()
            }
        }
        "volume_limit" => {
            if zh {
                "音量上限".into()
            } else {
                "Volume Limit".into()
            }
        }
        "enabled" => {
            if zh {
                "启用".into()
            } else {
                "Enabled".into()
            }
        }
        "open_mixer" => {
            if zh {
                "打开音量合成器".into()
            } else {
                "Open Volume Mixer".into()
            }
        }
        "open_sound" => {
            if zh {
                "打开声音设置".into()
            } else {
                "Open Sound Settings".into()
            }
        }
        "open_hotkey_settings" => {
            if zh {
                "打开快捷键设置".into()
            } else {
                "Open Hotkey Settings".into()
            }
        }
        "config_error" => {
            if zh {
                "打开配置文件夹失败".into()
            } else {
                "Failed to open config folder".into()
            }
        }
        "autostart" => {
            if zh {
                "开机自启".into()
            } else {
                "Auto Launch".into()
            }
        }
        "about" => {
            if zh {
                "关于".into()
            } else {
                "About".into()
            }
        }
        "exit" => {
            if zh {
                "退出".into()
            } else {
                "Exit".into()
            }
        }
        "chinese" => "中文".into(),
        "english" => "English".into(),
        "input_devices" => {
            if zh {
                "音频输入设备".into()
            } else {
                "Input Devices".into()
            }
        }
        "output_devices" => {
            if zh {
                "音频输出设备".into()
            } else {
                "Output Devices".into()
            }
        }
        "muted" => {
            if zh {
                "静音".into()
            } else {
                "Muted".into()
            }
        }
        "about_text" => {
            if zh {
                "Audio Switcher — 托盘音频切换工具\n纯 Rust 托盘工具\n\n右键菜单切换设备，中键静音，悬停滚轮调音量。".into()
            } else {
                "Audio Switcher — Tray audio switcher\nPure Rust tray tool\n\nRight-click to switch device, middle-click to mute, hover+wheel to adjust volume.".into()
            }
        }
        "refresh" => {
            if zh {
                "刷新设备列表".into()
            } else {
                "Refresh Devices".into()
            }
        }
        "language" => {
            if zh {
                "语言".into()
            } else {
                "Language".into()
            }
        }
        "system" => {
            if zh {
                "跟随系统".into()
            } else {
                "Follow System".into()
            }
        }
        "autostart_unknown" => {
            if zh {
                "开机自启（状态未知）".into()
            } else {
                "Auto Launch (status unknown)".into()
            }
        }
        "no_devices" => {
            if zh {
                "未检测到音频设备".into()
            } else {
                "No audio devices".into()
            }
        }
        "device_error" => {
            if zh {
                "切换设备失败".into()
            } else {
                "Failed to switch device".into()
            }
        }
        "input_error" => {
            if zh {
                "切换输入设备失败".into()
            } else {
                "Failed to switch input device".into()
            }
        }
        "mixer_error" => {
            if zh {
                "打开音量合成器失败".into()
            } else {
                "Failed to open volume mixer".into()
            }
        }
        "sound_error" => {
            if zh {
                "打开声音设置失败".into()
            } else {
                "Failed to open sound settings".into()
            }
        }
        _ => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Lang;

    #[test]
    fn i18n_zh_en() {
        assert_eq!(tr("mute", Lang::Zh), "全局静音");
        assert_eq!(tr("mute", Lang::En), "Mute");
        assert_eq!(tr("volume_limit", Lang::Zh), "音量上限");
        assert_eq!(tr("volume_limit", Lang::En), "Volume Limit");
        assert_eq!(tr("output_devices", Lang::Zh), "音频输出设备");
        assert_eq!(tr("output_devices", Lang::En), "Output Devices");
        assert_eq!(tr("input_devices", Lang::Zh), "音频输入设备");
        assert_eq!(tr("input_devices", Lang::En), "Input Devices");
    }

    #[test]
    fn hotkey_settings_labels() {
        assert_eq!(tr("open_hotkey_settings", Lang::Zh), "打开快捷键设置");
        assert_eq!(tr("open_hotkey_settings", Lang::En), "Open Hotkey Settings");
        assert_ne!(tr("config_error", Lang::Zh), "config_error");
        assert_ne!(tr("config_error", Lang::En), "config_error");
        assert_ne!(tr("no_devices", Lang::Zh), "no_devices");
        assert_ne!(tr("no_devices", Lang::En), "no_devices");
    }
}
