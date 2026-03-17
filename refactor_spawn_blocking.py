import re
import glob

# Step 1: change signatures in sqlite.rs
with open("src/server/storage/sqlite.rs", "r") as f:
    content = f.read()

# Replace `pub fn name(db: &Db, id: &str, ...)` with `pub async fn name(db: Db, id: String, ...)`
# Wait, this is hard to do perfectly with regex because of varying arguments.
