use pair_harness::types::{StatusJson, FilesChanged};

fn main() {
    let json_with_count = r#"{
      "ticket_id": "T-1",
      "github_issue": "https://github.com/The-AgenticFlow/template-todoapp/issues/1",
      "branch": "forge-1/T-1",
      "status": "IMPLEMENTATION_COMPLETE",
      "files_changed": 14
    }"#;
    
    let json_with_list = r#"{
      "ticket_id": "T-2",
      "status": "PR_OPENED",
      "branch": "forge-1/T-2",
      "files_changed": ["src/main.rs", "src/lib.rs"]
    }"#;
    
    let status1: StatusJson = serde_json::from_str(json_with_count)
        .expect("Failed to parse STATUS.json with count");
    println!("Parsed with count: status={}, ticket_id={}", status1.status, status1.ticket_id);
    println!("  files_changed: {:?}", status1.files_changed);
    
    let status2: StatusJson = serde_json::from_str(json_with_list)
        .expect("Failed to parse STATUS.json with list");
    println!("Parsed with list: status={}, ticket_id={}", status2.status, status2.ticket_id);
    println!("  files_changed: {:?}", status2.files_changed);
    
    println!("\n✅ Both formats parse successfully!");
}
