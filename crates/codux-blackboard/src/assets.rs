use crate::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

impl Store {
    pub fn save_image(
        &self,
        project: &str,
        title: &str,
        bytes: &[u8],
        actor: Actor,
    ) -> Result<Entry, String> {
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("blackboard.image.too_large".into());
        }
        let format = image::guess_format(bytes).map_err(|_| "blackboard.image.invalid")?;
        if !matches!(
            format,
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
                | image::ImageFormat::Bmp
        ) {
            return Err("blackboard.image.invalid".into());
        }
        let reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let (width, height) = reader
            .into_dimensions()
            .map_err(|_| "blackboard.image.invalid")?;
        if width == 0 || height == 0 || width as u64 * height as u64 > 64_000_000 {
            return Err("blackboard.image.too_large".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let dir = self
            .root
            .join("assets")
            .join(format!("{:x}", Sha256::digest(project.as_bytes())));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let filename = format!("{id}.{}", format.extensions_str()[0]);
        let path = dir.join(filename);
        let temp = dir.join(format!("{id}.tmp"));
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;
        match self.write(project,Component::Images,Some(&id),title,"",json!({"asset":path.file_name().unwrap().to_string_lossy(),"width":width,"height":height,"bytes":bytes.len(),"mime":format.to_mime_type(),"sha256":format!("{:x}",Sha256::digest(bytes))}),actor){Ok(e)=>Ok(e),Err(e)=>{let _=std::fs::remove_file(path);Err(e)}}
    }
    pub fn image_path(&self, project: &str, id: &str) -> Result<PathBuf, String> {
        let entry = self.read(project, Component::Images, id)?;
        let asset = entry
            .metadata
            .get("asset")
            .and_then(serde_json::Value::as_str)
            .ok_or("blackboard.image.unavailable")?;
        if Path::new(asset).components().count() != 1
            || asset.contains(['/', '\\', ':'])
            || Path::new(asset).file_name().and_then(|s| s.to_str()) != Some(asset)
        {
            return Err("blackboard.image.invalid".into());
        }
        let path = self
            .root
            .join("assets")
            .join(format!("{:x}", Sha256::digest(project.as_bytes())))
            .join(asset);
        if !path.is_file() {
            return Err("blackboard.image.unavailable".into());
        }
        Ok(path)
    }
    pub fn read_image(&self, project: &str, id: &str) -> Result<Vec<u8>, String> {
        std::fs::read(self.image_path(project, id)?).map_err(|e| e.to_string())
    }
    pub fn import_document(
        &self,
        project: &str,
        library: &str,
        path: &Path,
    ) -> Result<Entry, String> {
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !["md", "markdown", "txt"].contains(&extension.as_str()) {
            return Err("blackboard.document.invalid".into());
        }
        let mut data = Vec::new();
        std::fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
        if data.len() > 8 * 1024 * 1024 {
            return Err("blackboard.document.too_large".into());
        }
        let text = String::from_utf8(data).map_err(|_| "blackboard.document.encoding")?;
        let title = path.file_name().unwrap_or_default().to_string_lossy();
        self.import_text(project, library, &title, &text)
    }
    pub fn import_text(
        &self,
        project: &str,
        library: &str,
        title: &str,
        text: &str,
    ) -> Result<Entry, String> {
        let extension = Path::new(title)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if !["md", "markdown", "txt"].contains(&extension.as_str()) {
            return Err("blackboard.document.invalid".into());
        }
        let id = format!(
            "{:x}",
            Sha256::digest(format!("{library}/{title}").as_bytes())
        );
        self.write(
            project,
            Component::Knowledge,
            Some(&id),
            title,
            text.trim_start_matches('\u{feff}'),
            json!({"library":library,"format":extension}),
            Actor::user(),
        )
    }
    pub fn open_web(&self, project: &str, url: &str, actor: Actor) -> Result<Entry, String> {
        let parsed = url::Url::parse(url).map_err(|_| "blackboard.url.invalid")?;
        if !["http", "https"].contains(&parsed.scheme())
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("blackboard.url.invalid".into());
        }
        let normalized = parsed.to_string();
        let id = format!("{:x}", Sha256::digest(normalized.as_bytes()));
        let previous = self.read(project, Component::Web, &id).ok();
        let automatic = actor.automatic
            || actor
                .terminal_id
                .as_ref()
                .and_then(|id| self.terminal(project, id).ok())
                .is_some_and(|t| t.automatic);
        let page = self.write(
            project,
            Component::Web,
            Some(&id),
            parsed.host_str().unwrap_or(url),
            previous.as_ref().map_or("", |e| e.body.as_str()),
            json!({"url":normalized,"automatic":automatic,"openToken":uuid::Uuid::new_v4().to_string()}),
            actor,
        )?;
        let mut layout = self.snapshot(project)?.layout;
        layout.active_web = Some(id);
        self.save_layout(project, &layout)?;
        Ok(page)
    }
    pub fn fetch_web(&self, project: &str, id: &str) -> Result<Entry, String> {
        let mut entry = self.read(project, Component::Web, id)?;
        let url = entry
            .metadata
            .get("url")
            .and_then(serde_json::Value::as_str)
            .ok_or("blackboard.url.invalid")?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(8))
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client
            .get(url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("blackboard.web.too_large".into());
        }
        entry.body = html2text::from_read(bytes.as_slice(), 120).map_err(|e| e.to_string())?;
        Ok(entry)
    }
}
