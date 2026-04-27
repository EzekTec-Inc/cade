use super::*;

pub fn upsert_provider(db: &Db, row: &ProviderRow) -> Result<()> {
    let conn = db
        .lock();

    // SEC-02: Encrypt API key at rest
    let encrypted_key = match &row.api_key {
        Some(k) if !k.is_empty() => Some(crate::crypto::encrypt(k)?),
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
    let conn = db
        .lock();
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
        // previous DB-key file (pre-P2-1 `.cade-db.key`, or a different
        // `~/.cade/db.key`).  The provider will be re-created from env
        // vars if available; the user can also re-save it via /connect.
        let api_key = match encrypted_key {
            Some(k) if !k.is_empty() => {
                match crate::crypto::decrypt(&k) {
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
    let conn = db
        .lock();
    let n = conn.execute("DELETE FROM providers WHERE name = ?1", params![name])?;
    Ok(n > 0)
}

// region:    --- Tests

#[cfg(test)]
mod tests {
    #[allow(unused)]
    type Result<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    use super::*;

    fn setup_mem_db() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        apply_schema(&conn)?;
        run_migrations(&conn)?;
        Ok(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn test_upsert_and_list_providers() -> Result<()> {
        let db = setup_mem_db()?;
        assert!(list_providers(&db)?.is_empty());

        upsert_provider(
            &db,
            &ProviderRow {
                name: "ollama".into(),
                kind: "ollama".into(),
                api_key: None,
                base_url: Some("http://localhost:11434".into()),
                enabled: true,
            },
        )?;

        let providers = list_providers(&db)?;
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "ollama");
        assert_eq!(providers[0].kind, "ollama");
        assert!(providers[0].api_key.is_none());
        assert_eq!(providers[0].base_url, Some("http://localhost:11434".into()));
        Ok(())
    }

    #[test]
    fn test_upsert_provider_with_encrypted_key() -> Result<()> {
        let db = setup_mem_db()?;

        upsert_provider(
            &db,
            &ProviderRow {
                name: "anthropic".into(),
                kind: "anthropic".into(),
                api_key: Some("sk-test-key".into()),
                base_url: None,
                enabled: true,
            },
        )?;

        // The stored key should be encrypted (different from the plaintext)
        let providers = list_providers(&db)?;
        assert_eq!(providers.len(), 1);
        // list_providers decrypts, so we should get the original back
        assert_eq!(providers[0].api_key, Some("sk-test-key".into()));
        Ok(())
    }

    #[test]
    fn test_upsert_provider_update() -> Result<()> {
        let db = setup_mem_db()?;

        upsert_provider(
            &db,
            &ProviderRow {
                name: "test".into(),
                kind: "openai".into(),
                api_key: None,
                base_url: None,
                enabled: true,
            },
        )?;

        // Update same name with different kind
        upsert_provider(
            &db,
            &ProviderRow {
                name: "test".into(),
                kind: "anthropic".into(),
                api_key: None,
                base_url: None,
                enabled: false,
            },
        )?;

        let providers = list_providers(&db)?;
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].kind, "anthropic");
        Ok(())
    }

    #[test]
    fn test_delete_provider() -> Result<()> {
        let db = setup_mem_db()?;
        upsert_provider(
            &db,
            &ProviderRow {
                name: "test".into(),
                kind: "ollama".into(),
                api_key: None,
                base_url: None,
                enabled: true,
            },
        )?;
        assert!(delete_provider(&db, "test")?);
        assert!(list_providers(&db)?.is_empty());
        assert!(!delete_provider(&db, "nope")?);
        Ok(())
    }
}

// endregion: --- Tests
