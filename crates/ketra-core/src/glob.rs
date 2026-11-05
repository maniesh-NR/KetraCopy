//! Glob matching for `--include` / `--exclude` filters.
//!
//! Semantics (kept deliberately close to common shell globs):
//! - `*` matches any run of characters **not** crossing a `/` separator,
//! - `**` matches any run of characters including `/` (crosses directories),
//! - `?` matches exactly one character that is not `/`,
//! - `[abc]` / `[a-z]` character classes match one character,
//! - every other character matches itself.
//!
//! Patterns match against the `/`-separated relative path of a file.

/// Match `path` against glob `pattern`.
pub fn matches(pattern: &str, path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = path.replace('\\', "/").chars().collect();
    glob_match(&p, 0, &t, 0)
}

/// Match against any of the patterns (OR semantics).
pub fn matches_any(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| matches(p, path))
}

fn glob_match(p: &[char], mut pi: usize, t: &[char], mut ti: usize) -> bool {
    // Iterative backtracking with two backtrack points (see module docs):
    // * `single` — the last `*`, which may NOT absorb a `/`, and
    // * `double` — the last `**`, which may absorb anything.
    // On mismatch the single-star point advances first; when it is stuck on
    // a separator the double-star point takes over. This is what makes
    // `**/*.txt` match `a/b/c.txt` while `*.txt` does not.
    let mut single: Option<(usize, usize)> = None; // (pi_after_star, star_t)
    let mut double: Option<(usize, usize)> = None;

    while ti < t.len() {
        if pi < p.len() {
            match p[pi] {
                b if b == '*' => {
                    if pi + 1 < p.len() && p[pi + 1] == '*' {
                        // collapse '**…' and one following '/'
                        let mut np = pi + 2;
                        while np < p.len() && p[np] == '*' {
                            np += 1;
                        }
                        if np < p.len() && p[np] == '/' {
                            np += 1;
                        }
                        double = Some((np, ti));
                        pi = np;
                    } else {
                        single = Some((pi + 1, ti));
                        pi += 1;
                    }
                    continue;
                }
                '?' if t[ti] != '/' => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                '[' => {
                    if let Some(np) = class_match(p, pi, t[ti]) {
                        pi = np;
                        ti += 1;
                        continue;
                    }
                }
                c if c == t[ti] => {
                    pi += 1;
                    ti += 1;
                    continue;
                }
                _ => {}
            }
        }
        // Mismatch: let the single-star point absorb one more character.
        if let Some((sp, st)) = single {
            if t[st] != '/' {
                single = Some((sp, st + 1));
                pi = sp;
                ti = st + 1;
                continue;
            }
        }
        // Then the double-star point (absorbs anything, including '/').
        if let Some((sp, st)) = double {
            double = Some((sp, st + 1));
            pi = sp;
            ti = st + 1;
            continue;
        }
        return false;
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Try to match a `[...]` class at `p[pi]` against `c`.
/// Returns the pattern index after the class on success.
fn class_match(p: &[char], pi: usize, c: char) -> Option<usize> {
    let mut i = pi + 1;
    let mut negate = false;
    if i < p.len() && (p[i] == '!' || p[i] == '^') {
        negate = true;
        i += 1;
    }
    let mut hit = false;
    let mut first = true;
    while i < p.len() && (p[i] != ']' || first) {
        first = false;
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            if c >= p[i] && c <= p[i + 2] {
                hit = true;
            }
            i += 3;
        } else {
            if p[i] == c {
                hit = true;
            }
            i += 1;
        }
    }
    if i >= p.len() {
        return None; // unterminated class
    }
    let matched = if negate { !hit } else { hit };
    // '/' is never matched by a class, like '?'
    if c == '/' {
        return None;
    }
    if matched {
        Some(i + 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_star_does_not_cross_separator() {
        assert!(matches("*.txt", "a.txt"));
        assert!(matches("*.txt", "dir/a.txt") == false);
        assert!(matches("dir/*.txt", "dir/a.txt"));
        assert!(matches("dir/*.txt", "dir/sub/a.txt") == false);
    }

    #[test]
    fn test_double_star_crosses() {
        assert!(matches("**/*.txt", "a/b/c.txt"));
        assert!(matches("**", "a/b/c"));
        assert!(matches("a/**/c.txt", "a/b/c.txt"));
        assert!(matches("a/**/*.txt", "a/x.txt"));
    }

    #[test]
    fn test_question_mark_and_literals() {
        assert!(matches("file?.txt", "file1.txt"));
        assert!(matches("file?.txt", "file12.txt") == false);
        assert!(matches("file?.txt", "file/.txt") == false);
        assert!(matches("exact", "exact"));
        assert!(matches("exact", "exactx") == false);
    }

    #[test]
    fn test_character_class() {
        assert!(matches("file[0-9].txt", "file3.txt"));
        assert!(matches("file[0-9].txt", "filea.txt") == false);
        assert!(matches("file[!0-9].txt", "filea.txt"));
        assert!(matches("file[abc].txt", "fileb.txt"));
    }

    #[test]
    fn test_matches_any() {
        let pats: Vec<String> = vec!["*.jpg".into(), "*.png".into()];
        assert!(matches_any(&pats, "photo.png"));
        // A single `*` never crosses a separator — callers that want basename
        // matches pass the basename as well (see plan::allowed).
        assert!(matches_any(&pats, "deep/dir/photo.jpg") == false);
        assert!(matches_any(&["**/*.jpg".into()], "deep/dir/photo.jpg"));
        assert!(matches_any(&pats, "photo.gif") == false);
    }
}
