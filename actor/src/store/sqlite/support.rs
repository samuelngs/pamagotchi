use rusqlite::Connection;

pub(super) fn build_fts_query(input: &str) -> String {
    let words: Vec<&str> = input.split_whitespace().filter(|w| w.len() > 1).collect();
    if words.is_empty() {
        return input.to_string();
    }
    words
        .iter()
        .map(|w| {
            let clean: String = w.chars().filter(|c| c.is_alphanumeric()).collect();
            if clean.is_empty() {
                String::new()
            } else {
                format!("\"{clean}\"")
            }
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ")
}

pub(super) fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub(super) fn bytes_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub(super) struct TxGuard<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> TxGuard<'a> {
    pub(super) fn begin(conn: &'a Connection) -> anyhow::Result<Self> {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        Ok(Self { conn, done: false })
    }

    pub(super) fn commit(mut self) -> anyhow::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.done = true;
        Ok(())
    }
}

impl Drop for TxGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}
