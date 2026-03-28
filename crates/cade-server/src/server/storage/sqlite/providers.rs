use super::*;

pub fn upsert_provider(db: &Db, row: &ProviderRow) -> Result<()> {
    let conn = db.lock().expect("db lock poisoned");

    // SEC-02: Encrypt API key at rest
    let encrypted_key = match &row.api_key {
        Some(k) if !k.is_empty() => Some(crate::server::crypto::encrypt(k)?),
        other => other.clone(),
    };

    conn.execute(
        "INSERT INTO providers (name, kind, api_key, base_url, enabled, created_at)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(name) DO UPDATE SET
           kind    = excluded.kind,
           api_key = excluded.api_key,
           base_url= excluded.base_url,
           enabled = excluded.enabled",
        params![
            row.name,
            row.kind,
            encrypted_key,
            row.base_url,
            row.enabled as i64,
            now_ts(),
        ],
    )?;
    Ok(())
}

pub fn list_providers(db: &Db) -> Result<Vec<ProviderRow>> {
    let conn = db.lock().expect("db lock poisoned");
    let mut stmt =
        conn.prepare("SELECT name, kind, api_key, base_url, enabled FROM providers ORDER BY name")?;
    let mut providers = Vec::new();
    let mut rows = stmt.query([])?;

    while let Some(r) = rows.next()? {
        let name: String = r.get(0)?;
        let kind: String = r.get(1)?;
        let encrypted_key: Option<String> = r.get(2)?;
        let base_url: Option<String> = r.get(3)?;
        let enabled: bool = r.get::<_, i64>(4)? != 0;

        // SEC-02: Decrypt API key after retrieval
        // Decrypt API key — skip this provider on failure instead of aborting.
        // The key may have been encrypted with a different machine key or a
        // previous `.cade-db.key`.  The provider will be re-created from env
        // vars if available; the user can also re-save it via /connect.
        let api_key = match encrypted_key {
            Some(k) if !k.is_empty() => {
                match crate::server::crypto::decrypt(&k) {
                    Ok(d) => Some(d),
                    Err(e) => {
                        tracing::warn!(
                            "Skipping provider '{}': cannot decrypt API key ({e}). \
                             Re-save with /connect to re-encrypt with the current key.",
                            name
                        );
                        continue; // skip this row, load remaining providers
                    }
                }
            }
            other => other,
        };

        providers.push(ProviderRow {
            name,
            kind,
            api_key,
            base_url,
            enabled,
        });
    }

    Ok(providers)
}

pub fn delete_provider(db: &Db, name: &str) -> Result<bool> {
    let conn = db.lock().expect("db lock poisoned");
    let n = conn.execute("DELETE FROM providers WHERE name = ?1", params![name])?;
    Ok(n > 0)
}

