use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use protocol::{MediaAssetId, MediaKind};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::{MediaAsset, NewMediaAsset};

const DB_FILE: &str = "assets.sqlite";
const OBJECTS_DIR: &str = "objects";

pub struct MediaStore {
    root: PathBuf,
    conn: Mutex<Connection>,
}

impl MediaStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(OBJECTS_DIR))
            .with_context(|| format!("create media store root at {}", root.display()))?;

        let conn = Connection::open(root.join(DB_FILE)).with_context(|| {
            format!("open media store index at {}", root.join(DB_FILE).display())
        })?;
        init_schema(&conn)?;

        Ok(Self {
            root,
            conn: Mutex::new(conn),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put_bytes(&self, bytes: &[u8], asset: NewMediaAsset) -> Result<MediaAsset> {
        self.put_reader(Cursor::new(bytes), asset)
    }

    pub fn put_reader(&self, mut reader: impl Read, asset: NewMediaAsset) -> Result<MediaAsset> {
        let mut bytes = Vec::new();
        reader
            .read_to_end(&mut bytes)
            .context("read media payload")?;
        self.put_object(bytes, asset)
    }

    pub fn get(&self, id: &MediaAssetId) -> Result<Option<MediaAsset>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("media store index lock poisoned"))?;
        let values = conn
            .query_row(
                "SELECT id, kind, mime, filename, size, sha256, relative_path, metadata, created_at
                 FROM media_assets
                 WHERE id = ?1",
                params![id.0],
                row_values,
            )
            .optional()
            .context("load media asset")?;

        values
            .map(|values| self.asset_from_values(values))
            .transpose()
    }

    pub fn read_bytes(&self, id: &MediaAssetId) -> Result<Option<Vec<u8>>> {
        let Some(asset) = self.get(id)? else {
            return Ok(None);
        };

        fs::read(&asset.path)
            .with_context(|| format!("read media object at {}", asset.path.display()))
            .map(Some)
    }

    pub fn path_for(&self, id: &MediaAssetId) -> Result<Option<PathBuf>> {
        Ok(self.get(id)?.map(|asset| asset.path))
    }

    fn put_object(&self, bytes: Vec<u8>, asset: NewMediaAsset) -> Result<MediaAsset> {
        let sha256 = sha256_hex(&bytes);
        let relative_path = object_relative_path(&sha256)?;
        let object_path = self.root.join(&relative_path);

        if !object_path.exists() {
            let parent = object_path
                .parent()
                .ok_or_else(|| anyhow!("media object path has no parent"))?;
            fs::create_dir_all(parent)
                .with_context(|| format!("create media object directory {}", parent.display()))?;

            let tmp_path = parent.join(format!(".{}.{}.tmp", sha256, nanoid::nanoid!(8)));
            fs::write(&tmp_path, &bytes)
                .with_context(|| format!("write media object temp file {}", tmp_path.display()))?;
            fs::rename(&tmp_path, &object_path).with_context(|| {
                format!(
                    "commit media object {} to {}",
                    tmp_path.display(),
                    object_path.display()
                )
            })?;
        }

        let id = MediaAssetId(format!("media-{}", nanoid::nanoid!(12)));
        let created_at = current_unix_timestamp()?;
        let metadata =
            serde_json::to_string(&asset.metadata).context("serialize media metadata")?;
        let size = u64::try_from(bytes.len()).context("media payload size overflow")?;
        let size_i64 = i64::try_from(size).context("media payload too large for sqlite index")?;

        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow!("media store index lock poisoned"))?;
        conn.execute(
            "INSERT INTO media_assets
             (id, kind, mime, filename, size, sha256, relative_path, metadata, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id.0,
                asset.kind.as_str(),
                asset.mime,
                asset.filename,
                size_i64,
                sha256,
                relative_path.to_string_lossy(),
                metadata,
                created_at
            ],
        )
        .context("insert media asset index row")?;

        Ok(MediaAsset {
            id,
            kind: asset.kind,
            mime: asset.mime,
            filename: asset.filename,
            size,
            sha256,
            path: object_path,
            metadata: asset.metadata,
            created_at,
        })
    }

    fn asset_from_values(&self, values: AssetRow) -> Result<MediaAsset> {
        let kind = MediaKind::parse(&values.kind)
            .ok_or_else(|| anyhow!("unknown media kind in index: {}", values.kind))?;
        let size = u64::try_from(values.size)
            .with_context(|| format!("invalid media size for {}", values.id))?;
        let metadata = serde_json::from_str(&values.metadata)
            .with_context(|| format!("parse media metadata for {}", values.id))?;

        Ok(MediaAsset {
            id: MediaAssetId(values.id),
            kind,
            mime: values.mime,
            filename: values.filename,
            size,
            sha256: values.sha256,
            path: self.root.join(values.relative_path),
            metadata,
            created_at: values.created_at,
        })
    }
}

#[derive(Debug)]
struct AssetRow {
    id: String,
    kind: String,
    mime: Option<String>,
    filename: Option<String>,
    size: i64,
    sha256: String,
    relative_path: PathBuf,
    metadata: String,
    created_at: i64,
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS media_assets (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            mime TEXT,
            filename TEXT,
            size INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            relative_path TEXT NOT NULL,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_media_assets_sha256 ON media_assets(sha256);
        CREATE INDEX IF NOT EXISTS idx_media_assets_kind ON media_assets(kind);",
    )
    .context("initialize media store schema")
}

fn row_values(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetRow> {
    let relative_path: String = row.get(6)?;
    Ok(AssetRow {
        id: row.get(0)?,
        kind: row.get(1)?,
        mime: row.get(2)?,
        filename: row.get(3)?,
        size: row.get(4)?,
        sha256: row.get(5)?,
        relative_path: PathBuf::from(relative_path),
        metadata: row.get(7)?,
        created_at: row.get(8)?,
    })
}

fn current_unix_timestamp() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock before unix epoch")?;
    i64::try_from(elapsed.as_secs()).context("unix timestamp overflow")
}

fn object_relative_path(sha256: &str) -> Result<PathBuf> {
    if sha256.len() < 4 {
        bail!("sha256 digest is too short");
    }

    Ok(PathBuf::from(OBJECTS_DIR)
        .join(&sha256[0..2])
        .join(&sha256[2..4])
        .join(sha256))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);

    for byte in digest {
        output.push(hex_char(byte >> 4));
        output.push(hex_char(byte & 0x0f));
    }

    output
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        10..=15 => char::from(b'a' + nibble - 10),
        _ => unreachable!("nibble must be less than 16"),
    }
}

#[cfg(test)]
mod tests;
