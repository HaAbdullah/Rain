//! Search items of the three types that the launcher maps itself: sessions, jobs and idle jobs.
//! Section 01 6.3 (table of items).
//!
//! Sessions, jobs and idle jobs belong to section 03. That section offers plain lists
//! (`list()`) and no search trait, so the launcher turns each row into a `SearchItem` here.
//! These are pure functions: a row in, an item and its extra search words out.

use rain_types::{
    ClaudeSession, IconRef, IdleJob, IdleState, ScheduledJob, SearchItem, SearchType, Status,
};

use super::search::Candidate;

/// The last segment of a folder path. Windows paths use `\`, but `/` is accepted too.
fn folder_name(path: &str) -> &str {
    path.trim_end_matches(['\\', '/'])
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or("")
}

/// A session: the pending request ("Bash: npm test") or else the folder it works in.
pub fn session_candidate(session: &ClaudeSession) -> Candidate {
    let folder = folder_name(&session.cwd);
    let title = match &session.request {
        Some(request) => format!("{}: {}", request.tool, request.summary),
        None => folder.to_owned(),
    };
    let mut keywords = vec![folder.to_owned()];
    keywords.extend(session.branch.clone());
    Candidate {
        item: SearchItem {
            r#type: SearchType::Session,
            id: session.id.clone(),
            title,
            subtitle: Some(session.title.clone()),
            icon: IconRef::lucide("bot"),
            status: Some(session.status),
            score: 0,
        },
        keywords,
    }
}

/// A scheduled job: running now, else the status of its last run, else idle.
pub fn job_candidate(job: &ScheduledJob) -> Candidate {
    let status = if job.running {
        Status::Running
    } else {
        job.recent_runs
            .first()
            .map_or(Status::Idle, |run| run.status)
    };
    let kind = match job.kind {
        rain_types::JobKind::Cron => "cron",
        rain_types::JobKind::Claude => "claude",
    };
    Candidate {
        item: SearchItem {
            r#type: SearchType::Job,
            id: job.id.clone(),
            title: job.name.clone(),
            subtitle: Some(job.schedule_label.clone()),
            icon: IconRef::lucide("calendar-clock"),
            status: Some(status),
            score: 0,
        },
        keywords: vec![job.schedule_label.clone(), kind.to_owned()],
    }
}

/// An idle job: the subtitle is the state in words; queued and paused count as idle.
pub fn idle_job_candidate(job: &IdleJob) -> Candidate {
    let (label, status) = match job.state {
        IdleState::Queued => ("Queued", Status::Idle),
        IdleState::Running => ("Running", Status::Running),
        IdleState::Paused => ("Paused", Status::Idle),
        IdleState::Success => ("Done", Status::Success),
        IdleState::Failed => ("Failed", Status::Failed),
    };
    let resource = match job.resource {
        rain_types::IdleResource::Cpu => "cpu",
        rain_types::IdleResource::Gpu => "gpu",
    };
    Candidate {
        item: SearchItem {
            r#type: SearchType::IdleJob,
            id: job.id.clone(),
            title: job.name.clone(),
            subtitle: Some(label.to_owned()),
            icon: IconRef::lucide("cpu"),
            status: Some(status),
            score: 0,
        },
        keywords: vec![resource.to_owned()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_name_is_the_last_segment() {
        assert_eq!(folder_name(r"C:\Work\notes"), "notes");
        assert_eq!(folder_name(r"C:\Work\notes\"), "notes");
        assert_eq!(folder_name("/home/x/api"), "api");
        assert_eq!(folder_name(""), "");
    }
}
