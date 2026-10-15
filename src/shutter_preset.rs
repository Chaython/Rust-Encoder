//! Best-effort, read-only importer for Shutter Encoder's XML .enc presets.
//! Never execute embedded content or silently apply unsupported options.
use crate::model::{HardwareEncoder, Preset, Settings};
use std::{collections::HashMap, fs, path::Path};

pub struct ImportResult {
    pub settings: Settings,
    pub applied: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn import_file(path: &Path, base: &Settings) -> Result<ImportResult, String> {
    let metadata = fs::metadata(path).map_err(|e| format!("Cannot read preset: {e}"))?;
    if metadata.len() > 2_000_000 { return Err("Preset exceeds 2 MB limit".into()); }
    let xml = fs::read_to_string(path).map_err(|e| format!("Invalid preset text: {e}"))?;
    import_xml(&xml, base)
}

pub fn import_xml(xml: &str, base: &Settings) -> Result<ImportResult, String> {
    if xml.contains("<!DOCTYPE") || xml.contains("<!ENTITY") {
        return Err("XML entity declarations are not supported".into());
    }
    let document = roxmltree::Document::parse(xml).map_err(|e| format!("Invalid .enc XML: {e}"))?;
    let root = document.root_element();
    let is_settings = root.has_tag_name("Settings");
    let is_shutter_export = root.has_tag_name("Shutter")
        && root.children().any(|child| child.has_tag_name("Settings"));
    if !is_settings && !is_shutter_export {
        return Err("Not a Shutter Encoder settings XML document".into());
    }
    let mut values = HashMap::new();
    for component in document.descendants().filter(|n| n.has_tag_name("Component")) {
        let child = |name| component.children().find(|c| c.has_tag_name(name))
            .and_then(|c| c.text()).map(str::trim);
        if let (Some(name), Some(value)) = (child("Name"), child("Value")) {
            values.insert(name.to_owned(), value.to_owned());
        }
    }
    if values.is_empty() { return Err("No Shutter Encoder components found".into()); }
    let get = |key: &str| values.get(key).map(String::as_str);
    let mut settings = base.clone();
    let mut applied = Vec::new();
    let mut skipped = Vec::new();

    // The function must be supported; never misinterpret a different codec
    // as H.264 merely because that happens to be the default.
    let function = get("comboFonctions")
        .ok_or("Preset does not contain a comboFonctions entry")?;
    let preset = match function.trim().to_ascii_lowercase().as_str() {
        "h.264" | "h264" => Preset::H264,
        "h.265" | "h265" | "hevc" => Preset::H265,
        "av1" => Preset::Av1,
        "vp9" => Preset::Vp9,
        "apple prores" | "prores" => Preset::ProRes,
        "dnxhr" => Preset::Dnxhr,
        "flac" => Preset::Flac,
        "wav" => Preset::Wav,
        "mp3" => Preset::Mp3,
        "aac" => Preset::Aac,
        "opus" => Preset::Opus,
        _ => return Err(format!("Unsupported Shutter Encoder function: {function}")),
    };
    settings.preset = preset;
    applied.push(format!("Function: {function}"));
    // Legacy components store UI display strings. Keep only values we can
    // confidently translate and report the remainder to the user.
    if let Some(accel) = get("comboAccel") {
        let lower = accel.to_ascii_lowercase();
        let mapped = if lower.contains("nvidia") { Some(HardwareEncoder::Nvenc) }
            else if lower.contains("amd") { Some(HardwareEncoder::Amf) }
            else if lower.contains("intel") { Some(HardwareEncoder::Qsv) }
            else if lower.contains("videotoolbox") { Some(HardwareEncoder::VideoToolbox) }
            else if ["none", "aucune", "software", "cpu"].iter().any(|s| lower.contains(s)) {
                Some(HardwareEncoder::Software)
            } else { None };
        if let Some(hw) = mapped {
            if hw == HardwareEncoder::Software || hw.codec(preset).is_some() {
                settings.hardware_encoder = hw;
                applied.push(format!("Hardware: {accel}"));
            } else { skipped.push(format!("Hardware {accel} is incompatible with {function}")); }
        } else { skipped.push(format!("Unrecognized hardware choice: {accel}")); }
    }
    if let Some(value) = get("caseNormalizeAudio") {
        if value == "true" && (preset.video() || preset.audio_only()) {
            settings.normalize_audio = true;
            applied.push("Audio normalization".into());
        }
    }
    if let Some(value) = get("caseFastStart") {
        if matches!(preset, Preset::H264 | Preset::H265) {
            settings.faststart = value == "true";
            applied.push("MP4 faststart".into());
        }
    }
    if get("caseGOP") == Some("true") {
        if let Some(value) = get("gopSize") {
            match value.parse::<u16>() {
                Ok(n) if (1..=1000).contains(&n) && preset.video() => {
                    settings.keyframe_interval = n;
                    applied.push("GOP length".into());
                }
                _ => skipped.push(format!("Unsupported GOP length: {value}")),
            }
        }
    }
    if preset.video() && !matches!(preset, Preset::ProRes | Preset::Dnxhr) {
        if let Some(value) = get("debitVideo") {
            if let Ok(kbps) = value.replace(' ', "").parse::<u32>() {
                if (100..=100_000).contains(&kbps) {
                    settings.use_video_bitrate = true;
                    settings.video_bitrate_kbps = kbps;
                    applied.push("Target video bitrate".into());
                } else { skipped.push(format!("Video bitrate out of range: {value}")); }
            } else if value != "auto" {
                skipped.push(format!("Video bitrate mode not translated: {value}"));
            }
        }
        if settings.use_video_bitrate {
            if let Some(value) = get("maximumBitrate") {
                if let Ok(kbps) = value.replace(' ', "").parse::<u32>() {
                    if (settings.video_bitrate_kbps..=200_000).contains(&kbps) {
                        settings.max_video_bitrate_kbps = kbps;
                        applied.push("Peak video bitrate".into());
                    } else { skipped.push(format!("Peak bitrate incompatible with target: {value}")); }
                } else if value != "auto" {
                    skipped.push(format!("Peak bitrate not translated: {value}"));
                }
            }
        }
        if get("case2pass") == Some("true") {
            let supported = matches!(preset, Preset::H264 | Preset::H265 | Preset::Vp9)
                && settings.hardware_encoder == HardwareEncoder::Software
                && settings.use_video_bitrate;
            if supported {
                settings.two_pass = true;
                applied.push("Two-pass target bitrate".into());
            } else { skipped.push("Two-pass (unsupported with imported codec/mode)".into()); }
        }
    }
    if let Some(value) = get("debitAudio").or_else(|| get("comboAudioBitrate")) {
        let digits = value.split(|c: char| !c.is_ascii_digit()).next().unwrap_or("");
        if let Ok(kbps) = digits.parse::<u16>() {
            if (32..=512).contains(&kbps) {
                settings.audio_bitrate_kbps = kbps;
                applied.push("Audio bitrate".into());
            } else { skipped.push(format!("Unsupported audio bitrate: {value}")); }
        }
    }
    // The legacy UI stores codec overrides separately from the function.
    // Never quietly load a different audio encoder than the saved choice.
    if get("caseChangeAudioCodec") == Some("true") {
        if let Some(codec) = get("comboAudioCodec") {
            skipped.push(format!("Custom audio codec override requires manual review: {codec}"));
        } else {
            skipped.push("Custom audio codec override requires manual review".into());
        }
    }
    // Shutter's conform-FPS option is opt-in. Its fractional rates (e.g.
    // 23,98 and 29,97) cannot be represented by our integer FPS setting.
    // Never round them silently to a different cadence.
    if get("caseConform") == Some("true") {
        if !preset.video() {
            skipped.push("Frame-rate conform is unavailable for this function".into());
        } else if let Some(value) = get("comboFPS") {
            match value.trim().parse::<u16>() {
                Ok(fps) if (1..=240).contains(&fps) => {
                    settings.frame_rate = fps;
                    applied.push(format!("Conform frame rate: {fps} FPS"));
                }
                _ => skipped.push(format!("Fractional or unsupported conform FPS: {value}")),
            }
        } else {
            skipped.push("Conform FPS enabled without a stored frame rate".into());
        }
    }
    // Shutter's "caseSampleRate" specifically forces 48 kHz (lbl48k),
    // unlike the per-track routing selectors.
    if get("caseSampleRate") == Some("true") && (preset.video() || preset.audio_only()) {
        settings.audio_sample_rate = 48_000;
        applied.push("Force audio sample rate to 48 kHz".into());
    }
    // Explicitly map supported stream-copy functions.
    // "Rewrap" is container-dependent and cannot be inferred without a
    // saved output container; avoid guessing.
    if get("caseForcerProgressif") == Some("true") {
        skipped.push("Force progressive scan: no equivalent output-field-order control".into());
    }
    // Audio mapping is per-channel in Shutter. Arbitrary mixes are not
    // equivalent to selecting a single FFmpeg audio stream.
    // Rotation and horizontal mirroring have direct FFmpeg equivalents.
    if preset.video() {
        if get("caseRotate") == Some("true") {
            match get("comboRotate") {
                Some("0") => { settings.rotate_degrees = 0; applied.push("Rotation: 0°".into()); }
                Some("90") => { settings.rotate_degrees = 90; applied.push("Rotation: 90°".into()); }
                Some("-90") => { settings.rotate_degrees = 270; applied.push("Rotation: -90°".into()); }
                Some("180") => { settings.rotate_degrees = 180; applied.push("Rotation: 180°".into()); }
                Some(v) => skipped.push(format!("Unrecognized rotation: {v}")),
                None => skipped.push("Rotation enabled without angle".into()),
            }
        }
        if get("caseMiror") == Some("true") {
            settings.flip_horizontal = true;
            applied.push("Horizontal mirror".into());
        }
    }
    // Shutter Encoder's QMax checkbox controls quantizer constraints;
    // it is NOT an encoding-speed preset. Never substitute a speed setting.
    if get("caseQMax") == Some("true") && preset.video() {
        skipped.push("QMax quantizer constraints (not equivalent to encoder speed)".into());
    }
    if get("caseForcerDesentrelacement") == Some("true") && preset.video() {
        settings.deinterlace = true;
        applied.push("Deinterlacing".into());
    }
    // Catch additional non-checkbox features that are explicitly enabled but
    // have no equivalent yet. Saved Swing snapshots include disabled controls
    // and selected UI defaults; only active values should trigger warnings.
    const UNSUPPORTED_ACTIVE_OPTIONS: &[&str] = &[
        "caseAudioOffset", "caseAudioFadeIn", "caseAudioFadeOut",
        "caseVideoFadeIn", "caseVideoFadeOut", "caseMixAudio",
        "caseSplitAudio", "caseConvertAudioFramerate", "caseForceTune",
        "caseForceLevel", "caseForceOutput", "caseForcePreset",
        "caseForceQuality", "caseForceSpeed", "caseForcerEntrelacement",
        "caseForcerInversion", "caseForcerDAR", "caseSequenceFPS",
        "caseMixAudio",
    ];
    for name in UNSUPPORTED_ACTIVE_OPTIONS {
        if get(name) == Some("true") {
            skipped.push(format!("Unsupported enabled Shutter option: {name}"));
        }
    }
    // Shutter writes a large snapshot of Swing controls to each preset.
    // Report ALL active, unmapped checkboxes instead of silently losing
    // edits that are not in a small manually curated warning list.
    const MAPPED_CHECKBOXES: &[&str] = &[
        "caseNormalizeAudio", "caseFastStart", "caseGOP", "case2pass",
        "caseQMax", "caseForcerDesentrelacement", "caseConform", "caseSampleRate",
        "caseRotate", "caseMiror", "caseChangeAudioCodec",
        "caseAudioOffset", "caseAudioFadeIn", "caseAudioFadeOut",
        "caseVideoFadeIn", "caseVideoFadeOut", "caseMixAudio",
        "caseSplitAudio", "caseConvertAudioFramerate", "caseForceTune",
        "caseForceLevel", "caseForceOutput", "caseForcePreset",
        "caseForceQuality", "caseForceSpeed", "caseForcerEntrelacement",
        "caseForcerInversion", "caseForcerDAR", "caseSequenceFPS",
    ];
    for component in document.descendants().filter(|n| n.has_tag_name("Component")) {
        let child = |tag| component.children().find(|c| c.has_tag_name(tag))
            .and_then(|c| c.text()).map(str::trim);
        if child("Type") == Some("JCheckBox") && child("Value") == Some("true") {
            if let Some(name) = child("Name") {
                if !MAPPED_CHECKBOXES.contains(&name) {
                    skipped.push(format!("Unmapped enabled setting: {name}"));
                }
            }
        }
    }
    // Prevent combinations that Rust Encoder cannot run.
    if settings.hardware_encoder == HardwareEncoder::VideoToolbox {
        settings.use_video_bitrate = true;
    }
    if matches!(preset, Preset::ProRes | Preset::Dnxhr) {
        settings.use_video_bitrate = false;
        settings.max_video_bitrate_kbps = 0;
    }
    crate::encode::validate_options(&settings.encode_options())
        .map_err(|e| format!("Imported settings are incompatible: {e}"))?;
    Ok(ImportResult { settings, applied, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_known_xml_components_without_overwriting_base_paths() {
        let xml = r#"<Settings><Component><Type>JComboBox</Type><Name>comboFonctions</Name><Value>H.265</Value></Component><Component><Name>caseGOP</Name><Value>true</Value></Component><Component><Name>gopSize</Name><Value>60</Value></Component><Component><Name>comboAudioBitrate</Name><Value>192 kb/s</Value></Component></Settings>"#;
        let mut base = Settings::default();
        base.output_dir = "my-output-folder".into();
        let result = import_xml(xml, &base).unwrap();
        assert_eq!(result.settings.preset, Preset::H265);
        assert_eq!(result.settings.keyframe_interval, 60);
        assert_eq!(result.settings.audio_bitrate_kbps, 192);
        assert_eq!(result.settings.output_dir, "my-output-folder");
    }
    #[test]
    fn imports_numeric_bitrate_and_two_pass_from_legacy_controls() {
        let xml = r#"<Settings>
          <Component><Name>comboFonctions</Name><Value>H.264</Value></Component>
          <Component><Name>debitVideo</Name><Value>5000</Value></Component>
          <Component><Name>maximumBitrate</Name><Value>8000</Value></Component>
          <Component><Name>case2pass</Name><Value>true</Value></Component>
        </Settings>"#;
        let parsed = import_xml(xml, &Settings::default()).unwrap();
        assert!(parsed.settings.use_video_bitrate);
        assert_eq!(parsed.settings.video_bitrate_kbps, 5000);
        assert_eq!(parsed.settings.max_video_bitrate_kbps, 8000);
        assert!(parsed.settings.two_pass);
    }
    #[test]
    fn maps_audio_bitrate_and_deinterlacing_from_actual_legacy_controls() {
        let xml = r#"<Settings>
          <Component><Name>comboFonctions</Name><Value>H.264</Value></Component>
          <Component><Name>debitAudio</Name><Value>256</Value></Component>
          <Component><Name>caseForcerDesentrelacement</Name><Value>true</Value></Component>
        </Settings>"#;
        let result = import_xml(xml, &Settings::default()).unwrap();
        assert_eq!(result.settings.audio_bitrate_kbps, 256);
        assert!(result.settings.deinterlace);
    }
    #[test]
    fn conform_fps_imports_only_exact_integer_rates() {
        let xml = |fps: &str| format!(
            "<Settings><Component><Name>comboFonctions</Name><Value>H.264</Value></Component><Component><Name>caseConform</Name><Value>true</Value></Component><Component><Name>comboFPS</Name><Value>{fps}</Value></Component></Settings>");
        let integer = import_xml(&xml("60"), &Settings::default()).unwrap();
        assert_eq!(integer.settings.frame_rate, 60);
        let fractional = import_xml(&xml("59,94"), &Settings::default()).unwrap();
        assert_eq!(fractional.settings.frame_rate, 0);
        assert!(fractional.skipped.iter().any(|entry| entry.contains("59,94")));
        let disabled = xml("30").replace("<Value>true</Value>", "<Value>false</Value>");
        let no_conform = import_xml(&disabled, &Settings::default()).unwrap();
        assert_eq!(no_conform.settings.frame_rate, 0);
    }
    #[test]
    fn qmax_does_not_change_encoder_speed() {
        let xml = r#"<Settings>
            <Component><Name>comboFonctions</Name><Value>H.264</Value></Component>
            <Component><Name>caseQMax</Name><Value>true</Value></Component>
        </Settings>"#;
        let imported = import_xml(xml, &Settings::default()).unwrap();
        assert_eq!(imported.settings.encoder_speed, Settings::default().encoder_speed);
        assert!(imported.skipped.iter().any(|item| item.contains("QMax")));
    }
    #[test]
    fn reports_unmapped_enabled_checkboxes_but_not_disabled_ones() {
        let xml = r#"<Settings>
          <Component><Type>JComboBox</Type><Name>comboFonctions</Name><Value>H.264</Value></Component>
          <Component><Type>JCheckBox</Type><Name>caseWatermark</Name><Value>true</Value></Component>
          <Component><Type>JCheckBox</Type><Name>caseUnrelated</Name><Value>false</Value></Component>
          <Component><Type>JCheckBox</Type><Name>caseFastStart</Name><Value>true</Value></Component>
        </Settings>"#;
        let result = import_xml(xml, &Settings::default()).unwrap();
        assert!(result.skipped.iter().any(|s| s.contains("caseWatermark")));
        assert!(!result.skipped.iter().any(|s| s.contains("caseUnrelated")));
        assert!(!result.skipped.iter().any(|s| s.contains("caseFastStart")));
    }
    #[test]
    fn accepts_native_shutter_xml_wrapper() {
        let xml = "<Shutter><Settings><Component><Name>comboFonctions</Name><Value>H.264</Value></Component></Settings></Shutter>";
        let result = import_xml(xml, &Settings::default()).unwrap();
        assert_eq!(result.settings.preset, Preset::H264);
    }
    #[test]
    fn rejects_unrelated_xml_even_with_spoofed_component_names() {
        let xml = "<Unrelated><Component><Name>comboFonctions</Name><Value>H.264</Value></Component></Unrelated>";
        assert!(import_xml(xml, &Settings::default()).is_err());
    }
    #[test]
    fn warns_about_unsupported_active_audio_and_video_controls() {
        let xml = r#"<Shutter><Settings>
          <Component><Name>comboFonctions</Name><Value>H.264</Value></Component>
          <Component><Name>caseAudioOffset</Name><Value>true</Value></Component>
          <Component><Name>caseVideoFadeIn</Name><Value>true</Value></Component>
          <Component><Name>caseAudioFadeOut</Name><Value>false</Value></Component>
        </Settings></Shutter>"#;
        let result = import_xml(xml, &Settings::default()).unwrap();
        assert!(result.skipped.iter().any(|item| item.contains("caseAudioOffset")));
        assert!(result.skipped.iter().any(|item| item.contains("caseVideoFadeIn")));
        assert!(!result.skipped.iter().any(|item| item.contains("caseAudioFadeOut")));
    }
    #[test]
    fn imports_forced_48khz_audio_rate_only_when_enabled() {
        let xml = |enabled: bool| format!(
            "<Shutter><Settings><Component><Name>comboFonctions</Name><Value>H.264</Value></Component><Component><Type>JCheckBox</Type><Name>caseSampleRate</Name><Value>{enabled}</Value></Component></Settings></Shutter>");
        let active = import_xml(&xml(true), &Settings::default()).unwrap();
        assert_eq!(active.settings.audio_sample_rate, 48_000);
        assert!(!active.skipped.iter().any(|s| s.contains("caseSampleRate")));
        let inactive = import_xml(&xml(false), &Settings::default()).unwrap();
        assert_eq!(inactive.settings.audio_sample_rate, 0);
    }
    #[test]
    fn translates_shutter_rotation_and_mirror() {
        let xml = r#"<Shutter><Settings>
            <Component><Name>comboFonctions</Name><Value>H.264</Value></Component>
            <Component><Type>JCheckBox</Type><Name>caseRotate</Name><Value>true</Value></Component>
            <Component><Name>comboRotate</Name><Value>-90</Value></Component>
            <Component><Type>JCheckBox</Type><Name>caseMiror</Name><Value>true</Value></Component>
        </Settings></Shutter>"#;
        let imported = import_xml(xml, &Settings::default()).unwrap();
        assert_eq!(imported.settings.rotate_degrees, 270);
        assert!(imported.settings.flip_horizontal);
        assert!(!imported.skipped.iter().any(|v| v.contains("caseRotate") || v.contains("caseMiror")));
    }
    #[test]
    fn reports_custom_audio_codec_override_instead_of_guessing() {
        let xml = r#"<Shutter><Settings>
          <Component><Name>comboFonctions</Name><Value>H.264</Value></Component>
          <Component><Type>JCheckBox</Type><Name>caseChangeAudioCodec</Name><Value>true</Value></Component>
          <Component><Name>comboAudioCodec</Name><Value>AC3</Value></Component>
        </Settings></Shutter>"#;
        let result = import_xml(xml, &Settings::default()).unwrap();
        assert!(result.skipped.iter().any(|s| s.contains("AC3")));
    }
    #[test]
    fn refuses_unknown_function_and_entity_declaration() {
        let xml = "<Settings><Component><Name>comboFonctions</Name><Value>H.266</Value></Component></Settings>";
        assert!(import_xml(xml, &Settings::default()).is_err());
        assert!(import_xml("<!DOCTYPE a [<!ENTITY x SYSTEM 'file:///tmp/x'>]><a/>", &Settings::default()).is_err());
    }
}
