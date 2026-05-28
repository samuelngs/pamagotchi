use super::*;

#[test]
fn style_directive_schema_uses_adaptation_not_mirroring() {
    let tools = tools();
    let respond = tools
        .iter()
        .find(|tool| tool.name == "respond")
        .expect("respond tool exists");
    let description = respond.parameters["properties"]["style_directive"]["description"]
        .as_str()
        .expect("style_directive description exists");

    assert!(description.contains("adapt"));
    assert!(description.contains("without copying every quirk"));
    assert!(!description.contains("mirror"));
    assert!(!description.contains("mirrors"));
}
