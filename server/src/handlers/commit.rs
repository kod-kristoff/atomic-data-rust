use crate::{appstate::AppState, errors::AtomicServerResult};
use actix_web::{web, HttpResponse};
use atomic_lib::{commit::CommitOpts, parse::parse_json_ad_commit_resource, Commit, Storelike};

/// Send and process a Commit.
/// Currently only accepts JSON-AD
#[tracing::instrument(skip(appstate))]
pub async fn post_commit(
    appstate: web::Data<AppState>,
    body: String,
) -> AtomicServerResult<HttpResponse> {
    let store = &appstate.store;
    let mut builder = HttpResponse::Ok();
    let incoming_commit_resource = parse_json_ad_commit_resource(&body, store)?;
    let incoming_commit = Commit::from_resource(incoming_commit_resource)?;
    if store.is_external_subject(&incoming_commit.subject)? {
        return Err("Subject of commit is external, and should be sent to its origin domain. This store can not own this resource. See https://github.com/atomicdata-dev/atomic-data-rust/issues/509".into());
    }
    let opts = CommitOpts {
        validate_schema: true,
        validate_signature: true,
        validate_timestamp: true,
        validate_rights: true,
        // https://github.com/atomicdata-dev/atomic-data-rust/issues/412
        validate_previous_commit: false,
        validate_for_agent: Some(incoming_commit.signer.to_string()),
        update_index: true,
    };
    let commit_response = incoming_commit.apply_opts(store, &opts)?;

    let message = commit_response.commit_resource.to_json_ad()?;

    Ok(builder.body(message))
}
