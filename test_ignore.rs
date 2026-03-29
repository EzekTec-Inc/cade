use ignore::WalkBuilder;
fn main() {
    WalkBuilder::new(".").filter_entry(|e| e.file_name() != ".git");
}
