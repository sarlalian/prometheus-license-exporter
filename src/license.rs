pub fn is_excluded(excludes: &Option<Vec<String>>, feature: String) -> bool {
    let mut excluded: bool = false;

    if let Some(excl) = excludes {
        for f in excl {
            if *f == feature {
                excluded = true;
                break;
            }
        }
    }

    excluded
}
