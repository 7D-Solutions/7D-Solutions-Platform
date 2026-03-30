pub fn normalized_tags(tags: Option<Vec<String>>, existing: &[String]) -> Vec<String> {
    match tags {
        Some(list) => list,
        None => existing.to_vec(),
    }
}
