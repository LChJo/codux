use crate::{
    config::ConfigStore,
    settings::{
        AppSettings, UpdateSettings as AppUpdateSettings,
        app_settings::{effective_update_endpoint, is_parent_update_url},
    },
};
use semver::Version;
use serde::Serialize;
use serde_json::Value;
use std::{fs, path::PathBuf, time::Duration};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSummary {
    pub enabled: bool,
    pub channel: String,
    pub endpoint: String,
    pub latest_version: Option<String>,
    pub available: bool,
    pub platform_count: usize,
    pub notes_preview: String,
    pub error: Option<String>,
}

pub struct UpdateService {
    settings_path: PathBuf,
}

impl UpdateService {
    pub fn new(support_dir: PathBuf, _repo_root: PathBuf) -> Self {
        Self {
            settings_path: crate::config::settings_file_path(support_dir),
        }
    }

    pub fn summary(&self) -> UpdateSummary {
        let settings = self.settings();
        self.summary_for_update_settings(&settings, true)
    }

    pub fn settings_summary(&self) -> UpdateSummary {
        let settings = self.settings();
        self.summary_for_update_settings(&settings, false)
    }

    pub fn latest_release_version(&self) -> Result<String, String> {
        let settings = self.settings();
        let endpoint = effective_update_endpoint(&settings.endpoint, &settings.channel);
        let manifest = self.load_latest_manifest(&UpdateSummary {
            enabled: settings.enabled,
            channel: settings.channel,
            endpoint,
            ..Default::default()
        })?;
        if manifest["platforms"].as_object().is_some_and(|platforms| {
            !platforms.is_empty()
                && platforms
                    .values()
                    .all(|entry| entry["url"].as_str().is_some_and(is_parent_update_url))
        }) {
            return Err("No release has been published for this update channel.".to_string());
        }
        manifest_release_version(&manifest)
    }

    fn summary_for_update_settings(
        &self,
        settings: &AppUpdateSettings,
        load_manifest: bool,
    ) -> UpdateSummary {
        let mut summary = UpdateSummary {
            enabled: settings.enabled,
            channel: if settings.channel.is_empty() {
                "stable".to_string()
            } else {
                settings.channel.clone()
            },
            endpoint: effective_update_endpoint(&settings.endpoint, &settings.channel),
            ..Default::default()
        };
        if !load_manifest || !settings.enabled {
            return summary;
        }
        match self.load_latest_manifest(&summary) {
            Ok(value) => {
                summary.latest_version = value
                    .get("version")
                    .or_else(|| value.get("latestVersion"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                summary.available = update_status_from_manifest(
                    env!("CARGO_PKG_VERSION"),
                    summary.channel.clone(),
                    value.clone(),
                )
                .available;
                summary.platform_count = value
                    .get("platforms")
                    .and_then(Value::as_object)
                    .map(|platforms| platforms.len())
                    .unwrap_or(0);
                summary.notes_preview = manifest_notes(&value)
                    .as_deref()
                    .unwrap_or("")
                    .lines()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ");
            }
            Err(error) => summary.error = Some(error),
        }
        summary
    }

    pub fn status(&self, current_version: &str) -> UpdateStatus {
        let settings = self.settings();
        self.status_for_update_settings(&settings, current_version)
    }

    pub fn check_now(&self, current_version: &str) -> UpdateStatus {
        let mut settings = self.settings();
        settings.enabled = true;
        self.status_for_update_settings(&settings, current_version)
    }

    pub fn status_from_settings(
        settings: &AppSettings,
        repo_root: PathBuf,
        current_version: &str,
    ) -> UpdateStatus {
        Self::new(PathBuf::new(), repo_root)
            .status_for_update_settings(&settings.update, current_version)
    }

    pub fn status_for_update_settings(
        &self,
        settings: &AppUpdateSettings,
        current_version: &str,
    ) -> UpdateStatus {
        let endpoint = effective_update_endpoint(&settings.endpoint, &settings.channel);
        let endpoint_configured = settings.enabled && !endpoint.is_empty();
        if !endpoint_configured {
            return UpdateStatus {
                configured: false,
                checking: false,
                available: false,
                automatic_install_supported: false,
                signed_updater_configured: false,
                manifest_endpoint_configured: false,
                current_version: current_version.to_string(),
                latest_version: None,
                download_url: None,
                download_checksum: None,
                download_signature: None,
                notes: None,
                channel: Some(settings.channel.clone()).filter(|value| !value.trim().is_empty()),
                installation_mode: if settings.enabled {
                    "notConfigured".to_string()
                } else {
                    "disabled".to_string()
                },
                message: if settings.enabled {
                    "Unable to check the GitHub update channel for this build.".to_string()
                } else {
                    "Update checks are turned off.".to_string()
                },
            };
        }
        match self.load_latest_manifest(&UpdateSummary {
            enabled: settings.enabled,
            channel: settings.channel.clone(),
            endpoint,
            ..Default::default()
        }) {
            Ok(value) => {
                update_status_from_manifest(current_version, settings.channel.clone(), value)
            }
            Err(error) => UpdateStatus {
                configured: false,
                checking: false,
                available: false,
                automatic_install_supported: false,
                signed_updater_configured: false,
                manifest_endpoint_configured: true,
                current_version: current_version.to_string(),
                latest_version: None,
                download_url: None,
                download_checksum: None,
                download_signature: None,
                notes: None,
                channel: Some(settings.channel.clone()).filter(|value| !value.trim().is_empty()),
                installation_mode: "manualManifest".to_string(),
                message: format!("Unable to check updates: {error}"),
            },
        }
    }

    fn settings(&self) -> AppUpdateSettings {
        ConfigStore::for_file(self.settings_path.clone())
            .get_as::<AppUpdateSettings>("update")
            .unwrap_or_default()
    }

    fn load_latest_manifest(&self, settings: &UpdateSummary) -> Result<Value, String> {
        if let Some(path) = settings.endpoint.strip_prefix("file://") {
            return read_json_file(PathBuf::from(path));
        }
        if settings.endpoint.starts_with("http://") || settings.endpoint.starts_with("https://") {
            return fetch_json(&settings.endpoint);
        }
        if settings.endpoint.trim().is_empty() {
            return Err("Update endpoint is empty.".to_string());
        }
        read_json_file(PathBuf::from(&settings.endpoint))
    }
}

fn read_json_file(path: PathBuf) -> Result<Value, String> {
    let content = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&content).map_err(|error| error.to_string())
}

fn fetch_json(endpoint: &str) -> Result<Value, String> {
    crate::async_runtime::block_on(async move {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|error| error.to_string())?
            .get(endpoint)
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|error| error.to_string())?
            .json::<Value>()
            .await
            .map_err(|error| error.to_string())
    })
}

fn manifest_release_version(manifest: &Value) -> Result<String, String> {
    let version = manifest
        .get("version")
        .or_else(|| manifest.get("latestVersion"))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|version| version.trim_start_matches('v'))
        .filter(|version| !version.is_empty())
        .ok_or_else(|| "No release has been published for this update channel.".to_string())?;
    Version::parse(version)
        .map_err(|error| format!("Update manifest release version is invalid: {error}"))?;
    Ok(version.to_string())
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub configured: bool,
    pub checking: bool,
    pub available: bool,
    pub automatic_install_supported: bool,
    pub signed_updater_configured: bool,
    pub manifest_endpoint_configured: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub download_url: Option<String>,
    pub download_checksum: Option<String>,
    pub download_signature: Option<String>,
    pub notes: Option<String>,
    pub channel: Option<String>,
    pub installation_mode: String,
    pub message: String,
}

fn update_status_from_manifest(
    current_version: &str,
    channel: String,
    manifest: Value,
) -> UpdateStatus {
    let latest = manifest
        .get("version")
        .or_else(|| manifest.get("latestVersion"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let newer = latest
        .as_deref()
        .is_some_and(|version| version_is_newer(version, current_version));
    let latest_text = latest
        .clone()
        .unwrap_or_else(|| current_version.to_string());
    let platform_entry = current_platform_manifest_entry(&manifest);
    let download_signature = manifest_download_signature(platform_entry);
    let has_signed_updater =
        download_signature.is_some() && crate::app_info::updater_public_key().is_some();
    let signed_url = manifest_download_url(&manifest, platform_entry);
    let automatic_install_supported =
        has_signed_updater && platform_supports_automatic_install(signed_url.as_deref());
    let download_url = if automatic_install_supported {
        signed_url
    } else {
        manifest_entry_string(platform_entry, "manualUrl").or(signed_url)
    }
    .filter(|value| {
        !is_parent_update_url(value)
            && url::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "https" | "http"))
    });
    let available = newer && download_url.is_some();
    let download_checksum = manifest_download_checksum(&manifest, platform_entry);
    let automatic_install_supported = automatic_install_supported && download_url.is_some();
    let message = if available && automatic_install_supported {
        format!("A new version {latest_text} is available.")
    } else if available {
        format!(
            "A new version {latest_text} is available. Open the download URL to update manually."
        )
    } else if latest.is_none() {
        "No release has been published for this update channel.".to_string()
    } else if newer {
        format!("Version {latest_text} has no installer for this platform.")
    } else {
        format!("Current version {current_version} is up to date.")
    };
    UpdateStatus {
        configured: true,
        checking: false,
        available,
        automatic_install_supported,
        signed_updater_configured: automatic_install_supported,
        manifest_endpoint_configured: true,
        current_version: current_version.to_string(),
        latest_version: latest,
        download_url,
        download_checksum,
        download_signature,
        notes: manifest_notes(&manifest),
        channel: Some(channel).filter(|value| !value.trim().is_empty()),
        installation_mode: if automatic_install_supported {
            "automatic".to_string()
        } else {
            "manualManifest".to_string()
        },
        message,
    }
}

fn manifest_notes(manifest: &Value) -> Option<String> {
    [
        "notes",
        "releaseNotes",
        "release_notes",
        "changelog",
        "body",
        "description",
    ]
    .into_iter()
    .find_map(|key| {
        manifest
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn current_platform_manifest_entry(manifest: &Value) -> Option<&Value> {
    let platforms = manifest.get("platforms").and_then(Value::as_object)?;
    platform_keys_for_current_target()
        .iter()
        .filter_map(|key| platforms.get(*key))
        .next()
}

fn manifest_download_url(manifest: &Value, platform_entry: Option<&Value>) -> Option<String> {
    manifest_entry_string(platform_entry, "url").or_else(|| {
        manifest
            .get("downloadUrl")
            .or_else(|| manifest.get("download_url"))
            .or_else(|| manifest.get("url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn manifest_download_checksum(manifest: &Value, platform_entry: Option<&Value>) -> Option<String> {
    manifest_entry_string(platform_entry, "checksum").or_else(|| {
        manifest
            .get("checksum")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn manifest_download_signature(platform_entry: Option<&Value>) -> Option<String> {
    manifest_entry_string(platform_entry, "signature")
}

fn manifest_entry_string(entry: Option<&Value>, key: &str) -> Option<String> {
    entry
        .and_then(|entry| entry.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn platform_supports_automatic_install(download_url: Option<&str>) -> bool {
    #[cfg(target_os = "macos")]
    {
        download_url
            .map(str::to_ascii_lowercase)
            .is_some_and(|url| url.ends_with(".app.tar.gz"))
    }
    #[cfg(target_os = "windows")]
    {
        return download_url
            .map(str::to_ascii_lowercase)
            .is_some_and(|url| url.ends_with(".exe"));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = download_url;
        false
    }
}

fn platform_keys_for_current_target() -> &'static [&'static str] {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        &["darwin-aarch64-app", "darwin-aarch64"]
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        &["darwin-x86_64-app", "darwin-x86_64"]
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        &[
            "windows-x86_64",
            "windows-x86_64-nsis",
            "windows-x86_64-msi",
        ]
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        &["linux-x86_64"]
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64")
    )))]
    {
        &[]
    }
}

fn version_is_newer(latest: &str, current: &str) -> bool {
    let latest = latest.trim().trim_start_matches('v');
    let current = current.trim().trim_start_matches('v');
    match (Version::parse(latest), Version::parse(current)) {
        (Ok(latest), Ok(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_from_settings_reports_disabled_update_checks() {
        let mut settings = AppSettings::default();
        settings.update.enabled = false;

        let status = UpdateService::status_from_settings(&settings, PathBuf::new(), "1.0.0");

        assert!(!status.configured);
        assert!(!status.available);
        assert_eq!(status.installation_mode, "disabled");
        assert_eq!(status.message, "Update checks are turned off.");
    }

    #[test]
    fn status_from_settings_reads_local_manifest_endpoint() {
        let dir = std::env::temp_dir().join(format!(
            "codux-runtime-update-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let manifest_path = dir.join("latest.json");
        fs::write(
            &manifest_path,
            r#"{"version":"1.2.0","downloadUrl":"https://example.com/codux","notes":"new build"}"#,
        )
        .unwrap();

        let mut settings = AppSettings::default();
        settings.update.enabled = true;
        settings.update.channel = "stable".to_string();
        settings.update.endpoint = manifest_path.display().to_string();

        let status = UpdateService::status_from_settings(&settings, PathBuf::new(), "1.0.0");

        assert!(status.configured);
        assert!(status.available);
        assert_eq!(status.latest_version.as_deref(), Some("1.2.0"));
        assert_eq!(
            status.download_url.as_deref(),
            Some("https://example.com/codux")
        );
        assert_eq!(status.download_checksum, None);
        assert_eq!(status.installation_mode, "manualManifest");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_summary_does_not_load_manifest() {
        let dir = std::env::temp_dir().join(format!(
            "codux-runtime-update-settings-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let support_dir = dir.join("support");
        fs::create_dir_all(&support_dir).unwrap();
        let endpoint = dir.join("missing-latest.json").display().to_string();
        fs::write(
            support_dir.join("settings.json"),
            serde_json::json!({
                "update": {
                    "enabled": true,
                    "channel": "beta",
                    "endpoint": endpoint,
                }
            })
            .to_string(),
        )
        .unwrap();

        let service = UpdateService::new(support_dir, PathBuf::new());
        let summary = service.settings_summary();

        assert!(summary.enabled);
        assert_eq!(summary.channel, "beta");
        assert_eq!(summary.error, None);
        assert_eq!(summary.latest_version, None);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn manual_check_works_when_background_checks_are_disabled() {
        let dir =
            std::env::temp_dir().join(format!("codux-update-manual-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("latest.json");
        fs::write(&manifest, r#"{"version":"2.1.0","downloadUrl":"https://github.com/LChJo/codux/releases/download/v2.1.0/setup.exe"}"#).unwrap();
        fs::write(
            dir.join("settings.json"),
            serde_json::json!({ "update": { "enabled": false, "channel": "beta", "endpoint": manifest } }).to_string(),
        ).unwrap();
        let service = UpdateService::new(dir.clone(), PathBuf::new());
        assert!(!service.status("2.0.0").configured);
        let manual = service.check_now("2.0.0");
        assert!(manual.available);
        assert!(!manual.automatic_install_supported);
        assert!(!service.settings_summary().enabled);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_fork_manifest_does_not_offer_parent_installer() {
        let status = update_status_from_manifest(
            "2.0.0-beta.32",
            "beta".to_string(),
            serde_json::json!({
                "version": "2.0.3",
                "downloadUrl": "https://github.com/duxweb/codux/releases/download/v2.0.3/setup.exe",
            }),
        );
        assert!(!status.available);
        assert!(status.download_url.is_none());
    }

    #[test]
    fn latest_release_version_reads_and_normalizes_manifest_version() {
        let dir = std::env::temp_dir().join(format!(
            "codux-runtime-release-version-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        let support_dir = dir.join("support");
        fs::create_dir_all(&support_dir).unwrap();
        let manifest_path = dir.join("latest.json");
        fs::write(&manifest_path, r#"{"version":"v2.0.0-rc.10"}"#).unwrap();
        fs::write(
            support_dir.join("settings.json"),
            serde_json::json!({
                "update": {
                    "enabled": false,
                    "channel": "stable",
                    "endpoint": manifest_path.display().to_string(),
                }
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(
            UpdateService::new(support_dir, PathBuf::new())
                .latest_release_version()
                .unwrap(),
            "2.0.0-rc.10"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn status_reads_current_platform_download_url_from_platform_manifest() {
        let status = update_status_from_manifest(
            "1.0.0",
            "stable".to_string(),
            serde_json::json!({
                "version": "1.2.0",
                "platforms": {
                    platform_keys_for_current_target().first().copied().unwrap_or("unknown"): {
                        "url": "https://example.com/codux-platform",
                        "checksum": "abc"
                    }
                }
            }),
        );

        assert_eq!(
            status.available,
            !platform_keys_for_current_target().is_empty()
        );
        if !platform_keys_for_current_target().is_empty() {
            assert_eq!(
                status.download_url.as_deref(),
                Some("https://example.com/codux-platform")
            );
            assert_eq!(status.download_checksum.as_deref(), Some("abc"));
        }
        assert_eq!(status.installation_mode, "manualManifest");
    }

    #[test]
    fn status_prefers_platform_download_url_over_manual_download_url() {
        let platform_key = platform_keys_for_current_target()
            .first()
            .copied()
            .unwrap_or("unknown");
        let status = update_status_from_manifest(
            "1.0.0",
            "stable".to_string(),
            serde_json::json!({
                "version": "1.2.0",
                "downloadUrl": "https://example.com/codux-manual.dmg",
                "checksum": "manual-checksum",
                "platforms": {
                    platform_key: {
                        "url": "https://example.com/codux-platform.app.zip",
                        "checksum": "platform-checksum"
                    }
                }
            }),
        );

        if !platform_keys_for_current_target().is_empty() {
            assert_eq!(
                status.download_url.as_deref(),
                Some("https://example.com/codux-platform.app.zip")
            );
            assert_eq!(
                status.download_checksum.as_deref(),
                Some("platform-checksum")
            );
        }
    }

    #[test]
    fn automatic_install_requires_supported_platform_asset() {
        let platform_key = platform_keys_for_current_target()
            .first()
            .copied()
            .unwrap_or("unknown");
        let download_url = if cfg!(target_os = "windows") {
            "https://example.com/Codux-setup.exe"
        } else {
            "https://example.com/Codux.app.tar.gz"
        };
        let status = update_status_from_manifest(
            "1.0.0",
            "stable".to_string(),
            serde_json::json!({
                "version": "1.2.0",
                "platforms": {
                    platform_key: {
                        "url": download_url,
                        "signature": "signed"
                    }
                }
            }),
        );

        assert_eq!(
            status.automatic_install_supported,
            crate::app_info::updater_public_key().is_some()
                && (cfg!(target_os = "macos") || cfg!(target_os = "windows"))
                && !platform_keys_for_current_target().is_empty()
        );
    }

    #[test]
    fn automatic_install_requires_signed_tauri_platform_entry() {
        let platform_key = platform_keys_for_current_target()
            .first()
            .copied()
            .unwrap_or("unknown");
        let download_url = if cfg!(target_os = "windows") {
            "https://example.com/Codux-setup.exe"
        } else {
            "https://example.com/Codux.app.tar.gz"
        };
        let status = update_status_from_manifest(
            "1.0.0",
            "stable".to_string(),
            serde_json::json!({
                "version": "1.2.0",
                "platforms": {
                    platform_key: {
                        "url": download_url,
                        "signature": ""
                    }
                }
            }),
        );

        assert!(!status.automatic_install_supported);
        assert_eq!(status.installation_mode, "manualManifest");
    }

    #[test]
    fn checked_in_update_manifests_only_offer_fork_downloads() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .expect("workspace root")
            .to_path_buf();
        for channel in ["beta", "stable"] {
            let path = root.join("updates").join(channel).join("latest.json");
            let manifest: Value = serde_json::from_slice(&fs::read(&path).expect("manifest"))
                .expect("valid manifest");
            let platforms = manifest["platforms"]
                .as_object()
                .expect("manifest platforms");
            if !platforms.is_empty() {
                assert!(manifest_release_version(&manifest).is_ok());
            }
            for (platform, entry) in platforms {
                for key in ["url", "manualUrl"] {
                    if let Some(url) = entry[key].as_str() {
                        assert!(
                            url.starts_with("https://github.com/LChJo/codux/releases/download/")
                        );
                        if platform.starts_with("windows-") {
                            assert!(url.ends_with("/codux-windows-x86_64-setup.exe"));
                        }
                    }
                }
            }
        }
    }
}
