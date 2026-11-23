//! Pools: the rules for a draft from the editor, and the id of a new pool. Section 01 6.7.
//!
//! The same limits check `pools.toml` when the user edits it by hand (`store::validate_pools`);
//! here they become the fixed texts that the dialog shows under the form.

use std::collections::HashSet;

use rain_types::PoolDraft;

use super::apps::{first_free, fnv1a, slug};
use super::limits::{ARGS_MAX, DELAY_MAX_MS, DELAY_SUM_MAX_MS, POOL_MEMBERS_MAX, POOL_NAME_MAX};
use super::store::{MemberRow, is_plain_id};
use crate::core::error::ApiError;

/// A draft that passed: the trimmed name and the members as the file stores them.
pub struct CheckedDraft {
    pub name: String,
    pub members: Vec<MemberRow>,
}

fn invalid(reason: &'static str, message: &'static str) -> ApiError {
    ApiError::invalid(message)
        .with_source("launcher")
        .with_reason(reason)
}

/// Checks a draft. The first rule that fails decides the answer.
pub fn check_draft(draft: &PoolDraft) -> Result<CheckedDraft, ApiError> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err(invalid("emptyName", "A pool needs a name"));
    }
    if name.chars().count() > POOL_NAME_MAX {
        return Err(invalid(
            "nameTooLong",
            "A pool name can have at most 80 characters",
        ));
    }
    if draft.members.is_empty() {
        return Err(invalid("noMembers", "A pool needs at least one app"));
    }
    if draft.members.len() > POOL_MEMBERS_MAX {
        return Err(invalid("tooManyMembers", "A pool can hold at most 40 apps"));
    }
    let mut delays = 0u32;
    let mut members = Vec::with_capacity(draft.members.len());
    for member in &draft.members {
        let args = member.args.as_deref().filter(|args| !args.is_empty());
        let args_ok = args
            .is_none_or(|args| args.chars().count() <= ARGS_MAX && !args.contains(['\n', '\r']));
        let delay = member.delay_ms.unwrap_or(0);
        if !is_plain_id(&member.app_id) || !args_ok || delay > DELAY_MAX_MS {
            return Err(invalid("badMember", "A pool member is not valid"));
        }
        delays = delays.saturating_add(delay);
        members.push(MemberRow {
            app: member.app_id.clone(),
            args: args.map(str::to_owned),
            delay_ms: member.delay_ms,
        });
    }
    if delays > DELAY_SUM_MAX_MS {
        return Err(invalid(
            "delaysTooLong",
            "The delays of a pool add up to more than 10 minutes",
        ));
    }
    Ok(CheckedDraft {
        name: name.to_owned(),
        members,
    })
}

/// The id of a new pool: `pool-` plus the slug of the name, with `-2`, `-3` on a clash. A name
/// with no ASCII letter or digit uses 8 hex digits of a hash of the name and the pool count.
pub fn new_pool_id(name: &str, existing: &HashSet<String>) -> String {
    let slug = slug(name, 40);
    let base = if slug.is_empty() {
        let hash = fnv1a(&format!("{name}|{}", existing.len()));
        format!("pool-{:08x}", hash as u32)
    } else {
        format!("pool-{slug}")
    };
    first_free(&base, existing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rain_types::PoolMember;

    fn member(app: &str) -> PoolMember {
        PoolMember {
            app_id: app.into(),
            args: None,
            delay_ms: None,
        }
    }

    fn draft(name: &str, members: Vec<PoolMember>) -> PoolDraft {
        PoolDraft {
            id: None,
            name: name.into(),
            members,
        }
    }

    fn reason_of(draft: &PoolDraft) -> (String, String) {
        let error = check_draft(draft).err().expect("the draft is refused");
        (error.body.reason.unwrap_or_default(), error.body.message)
    }

    #[test]
    fn a_good_draft_is_trimmed_and_empty_args_are_dropped() {
        let mut first = member("app-a");
        first.args = Some(String::new());
        let checked = check_draft(&draft("  Web dev  ", vec![first, member("app-b")])).unwrap();
        assert_eq!(checked.name, "Web dev");
        assert_eq!(checked.members[0].args, None);
        assert_eq!(checked.members[1].app, "app-b");
    }

    #[test]
    fn every_rule_has_its_reason_and_text() {
        assert_eq!(
            reason_of(&draft("  ", vec![member("a")])),
            ("emptyName".into(), "A pool needs a name".into())
        );
        assert_eq!(
            reason_of(&draft(&"x".repeat(81), vec![member("a")])).0,
            "nameTooLong"
        );
        assert_eq!(
            reason_of(&draft("A", vec![])),
            ("noMembers".into(), "A pool needs at least one app".into())
        );
        assert_eq!(
            reason_of(&draft("A", vec![member("a"); 41])).0,
            "tooManyMembers"
        );
        assert_eq!(reason_of(&draft("A", vec![member("a b")])).0, "badMember");
        let mut with_break = member("a");
        with_break.args = Some("x\ny".into());
        assert_eq!(reason_of(&draft("A", vec![with_break])).0, "badMember");
        let mut too_long = member("a");
        too_long.args = Some("x".repeat(2001));
        assert_eq!(reason_of(&draft("A", vec![too_long])).0, "badMember");
        let mut late = member("a");
        late.delay_ms = Some(60_001);
        assert_eq!(reason_of(&draft("A", vec![late])).0, "badMember");
        let slow = |ms| PoolMember {
            delay_ms: Some(ms),
            ..member("a")
        };
        let many: Vec<PoolMember> = (0..11).map(|_| slow(60_000)).collect();
        assert_eq!(
            reason_of(&draft("A", many)),
            (
                "delaysTooLong".into(),
                "The delays of a pool add up to more than 10 minutes".into()
            )
        );
        // Exactly ten minutes is allowed.
        let ten: Vec<PoolMember> = (0..10).map(|_| slow(60_000)).collect();
        assert!(check_draft(&draft("A", ten)).is_ok());
    }

    #[test]
    fn a_new_pool_id_is_a_slug_with_suffixes_on_a_clash() {
        let mut taken: HashSet<String> = HashSet::new();
        assert_eq!(new_pool_id("Web dev", &taken), "pool-web-dev");
        taken.insert("pool-web-dev".into());
        assert_eq!(new_pool_id("Web Dev!", &taken), "pool-web-dev-2");
        let hashed = new_pool_id("\u{e9}\u{e9}", &taken);
        assert!(
            hashed.starts_with("pool-") && hashed.len() == 13,
            "{hashed}"
        );
    }
}
