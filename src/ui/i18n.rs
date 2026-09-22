//! Internationalisation helpers.
//!
//! `tr` translates a message key for the given [`Lang`].

use crate::config::Lang;

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
        "autostart_off" => {
            if zh {
                "关闭".into()
            } else {
                "Off".into()
            }
        }
        "autostart_user" => {
            if zh {
                "普通权限".into()
            } else {
                "Standard".into()
            }
        }
        "autostart_admin" => {
            if zh {
                "管理员权限".into()
            } else {
                "Administrator".into()
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
mod tests;
