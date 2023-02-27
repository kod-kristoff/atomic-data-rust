//! Functions useful in the server

use actix_web::cookie::Cookie;
use actix_web::http::header::{HeaderMap, HeaderValue};
use actix_web::http::Uri;
use atomic_lib::authentication::AuthValues;
use atomic_lib::AtomicError;
use percent_encoding::percent_decode_str;
use std::str::FromStr;

use crate::errors::{AppErrorType, AtomicServerError};
use crate::{appstate::AppState, content_types::ContentType, errors::AtomicServerResult};

/// Returns the authentication headers from the request
#[tracing::instrument(skip_all)]
pub fn get_auth_headers(
    map: &HeaderMap,
    requested_subject: String,
) -> AtomicServerResult<Option<AuthValues>> {
    let public_key = map.get("x-atomic-public-key");
    let signature = map.get("x-atomic-signature");
    let timestamp = map.get("x-atomic-timestamp");
    let agent = map.get("x-atomic-agent");
    match (public_key, signature, timestamp, agent) {
        (Some(pk), Some(sig), Some(ts), Some(a)) => Ok(Some(AuthValues {
            public_key: pk
                .to_str()
                .map_err(|_e| "Only string headers allowed")?
                .to_string(),
            signature: sig
                .to_str()
                .map_err(|_e| "Only string headers allowed")?
                .to_string(),
            agent_subject: a
                .to_str()
                .map_err(|_e| "Only string headers allowed")?
                .to_string(),
            timestamp: ts
                .to_str()
                .map_err(|_e| "Only string headers allowed")?
                .parse::<i64>()
                .map_err(|_e| "Timestamp must be a number (milliseconds since unix epoch)")?,
            requested_subject,
        })),
        (None, None, None, None) => Ok(None),
        _missing => Err("Missing authentication headers. You need `x-atomic-public-key`, `x-atomic-signature`, `x-atomic-agent` and `x-atomic-timestamp` for authentication checks.".into()),
    }
}

fn origin(url: &str) -> String {
    let parsed = Uri::from_str(url).unwrap();

    format!(
        "{}://{}",
        parsed.scheme_str().unwrap(),
        parsed.authority().unwrap()
    )
}

pub fn get_auth_from_cookie(
    map: &HeaderMap,
    requested_subject: &str,
) -> AtomicServerResult<Option<AuthValues>> {
    let encoded_session_cookies = match map.get("Cookie") {
        Some(cookies) => session_cookies_from_header(cookies)?,
        None => return Ok(None),
    };

    if encoded_session_cookies.is_empty() {
        return Ok(None);
    }
    // if there are multiple session cookies, we can try multiple
    let check_multiple = encoded_session_cookies.len() > 1;

    let mut err: AtomicServerError =
        AtomicError::unauthorized("No valid session cookies found. ".into()).into();

    for enc in encoded_session_cookies {
        let session = base64::decode(enc).map_err(|_| {
            AtomicError::unauthorized(
                "Malformed authentication resource - unable to decode base64".to_string(),
            )
        })?;

        let session_str = std::str::from_utf8(&session).map_err(|_| AtomicServerError {
            message: "Malformed authentication resource - unable to parse from utf_8".to_string(),
            error_type: AppErrorType::Unauthorized,
            error_resource: None,
        })?;
        let auth_values: AuthValues =
            serde_json::from_str(session_str).map_err(|e| AtomicServerError {
                message: format!(
                    "Malformed authentication resource when parsing AuthValues JSON: {}",
                    e
                ),
                error_type: AppErrorType::Unauthorized,
                error_resource: None,
            })?;

        let subject_invalid = auth_values.requested_subject.ne(requested_subject)
            && auth_values.requested_subject.ne(&origin(requested_subject));

        if subject_invalid {
            // if the subject is invalid, there are two things that could be going on.
            // 1. The requested resource is wrong
            // 2. The user is trying to access a resource from a different origin

            err = AtomicError::unauthorized(format!(
                "Wrong requested subject, expected {} was {}",
                requested_subject, auth_values.requested_subject
            ))
            .into();
            if check_multiple {
                continue;
            } else {
                return Err(err);
            }
        }
        return Ok(Some(auth_values));
    }
    Err(err)
}

pub fn get_auth(
    map: &HeaderMap,
    requested_subject: String,
) -> AtomicServerResult<Option<AuthValues>> {
    let from_header = match get_auth_headers(map, requested_subject.clone()) {
        Ok(res) => res,
        Err(err) => return Err(err),
    };

    match from_header {
        Some(v) => Ok(Some(v)),
        None => get_auth_from_cookie(map, &requested_subject),
    }
}

/// Checks for authentication headers and returns Some agent's subject if everything is well.
/// Skips these checks in public_mode and returns Ok(None).
/// Returns the Agent's subject or the Public Agent.
#[tracing::instrument(skip(appstate))]
pub fn get_client_agent(
    headers: &HeaderMap,
    appstate: &AppState,
    requested_subject: String,
) -> AtomicServerResult<Option<String>> {
    if appstate.config.opts.public_mode {
        return Ok(None);
    }
    // Authentication check. If the user has no headers, continue with the Public Agent.
    let auth_header_values = get_auth(headers, requested_subject)?;
    let for_agent = atomic_lib::authentication::get_agent_from_auth_values_and_check(
        auth_header_values,
        &appstate.store,
    )
    .map_err(|e| format!("Authentication failed: {}", e))?;
    Ok(Some(for_agent))
}

/// Finds the extension
pub fn try_extension(path: &str) -> Option<(ContentType, &str)> {
    let items: Vec<&str> = path.split('.').collect();
    if items.len() == 2 {
        let path = items[0];
        let content_type = match items[1] {
            "json" => ContentType::Json,
            "jsonld" => ContentType::JsonLd,
            "jsonad" => ContentType::JsonAd,
            "html" => ContentType::Html,
            "ttl" => ContentType::Turtle,
            _ => return None,
        };
        return Some((content_type, path));
    }
    None
}

fn session_cookies_from_header(header: &HeaderValue) -> AtomicServerResult<Vec<String>> {
    let cookies: Vec<&str> = header
        .to_str()
        .map_err(|_| "Can't convert header value to string")?
        .split(';')
        .collect();

    let mut found = Vec::new();

    for encoded_cookie in cookies {
        let cookie = Cookie::parse(encoded_cookie).map_err(|_| "Can't parse cookie")?;
        if cookie.name() == "atomic_session" {
            let decoded = percent_decode_str(cookie.value())
                .decode_utf8()
                .map_err(|_| "Can't decode cookie string")?;
            found.push(decoded.into());
        }
    }

    Ok(found)
}

#[test]
fn parse_cookie() {
    let cookie = "atomic_session=eyJodHRwczovL2F0b21pY2RhdGEuZGV2L3Byb3BlcnRpZXMvYXV0aC9hZ2VudCI6Imh0dHA6Ly9sb2NhbGhvc3Q6OTg4My9hZ2VudHMvaGVua2llcGVuayIsImh0dHBzOi8vYXRvbWljZGF0YS5kZXYvcHJvcGVydGllcy9hdXRoL3JlcXVlc3RlZFN1YmplY3QiOiJodHRwOi8vbG9jYWxob3N0Ojk4ODMiLCJodHRwczovL2F0b21pY2RhdGEuZGV2L3Byb3BlcnRpZXMvYXV0aC9wdWJsaWNLZXkiOiJLM3hsa0UxQmFIVXNnRzlYT0h4MVZaVUQ1TGs3ODJua09UcDVHNFN0SDdBPSIsImh0dHBzOi8vYXRvbWljZGF0YS5kZXYvcHJvcGVydGllcy9hdXRoL3RpbWVzdGFtcCI6MTY3NjI4MTU1NjEyNCwiaHR0cHM6Ly9hdG9taWNkYXRhLmRldi9wcm9wZXJ0aWVzL2F1dGgvc2lnbmF0dXJlIjoiMlprdFFWNTNkMVhNUWp4YklSN1pYRkhCMExGT2hHcVlpVlEyRENWc3BkZHVuL3ZHRkhJN3lqdU5jRitIMmpLa0Y0L0R4amEraHdTeUJlZ2ZvTWlxQ1E9PSJ9";

    let mut headermap = HeaderMap::new();
    headermap.insert(
        "Cookie".try_into().unwrap(),
        HeaderValue::from_str(cookie).unwrap(),
    );
    let subject = "http://localhost:9883";
    let out = get_auth_from_cookie(&headermap, subject)
        .expect("Should not return err")
        .expect("Should contain cookie");

    assert_eq!(out.requested_subject, subject);
}

#[test]
fn mutliple_auth_cookies() {
    let cookie = "atomic_session=eyJodHRwczovL2F0b21pY2RhdGEuZGV2L3Byb3BlcnRpZXMvYXV0aC9hZ2VudCI6Imh0dHBzOi8vYXRvbWljZGF0YS5kZXYvYWdlbnRzL1FtZnBSSUJuMkpZRWF0VDBNalNrTU5vQkp6c3R6MTlvcnduVDVvVDJyY1E9IiwiaHR0cHM6Ly9hdG9taWNkYXRhLmRldi9wcm9wZXJ0aWVzL2F1dGgvcmVxdWVzdGVkU3ViamVjdCI6Imh0dHBzOi8vYXRvbWljZGF0YS5kZXYiLCJodHRwczovL2F0b21pY2RhdGEuZGV2L3Byb3BlcnRpZXMvYXV0aC9wdWJsaWNLZXkiOiJRbWZwUklCbjJKWUVhdFQwTWpTa01Ob0JKenN0ejE5b3J3blQ1b1QycmNRPSIsImh0dHBzOi8vYXRvbWljZGF0YS5kZXYvcHJvcGVydGllcy9hdXRoL3RpbWVzdGFtcCI6MTY3NjI4MjU4NDg0NCwiaHR0cHM6Ly9hdG9taWNkYXRhLmRldi9wcm9wZXJ0aWVzL2F1dGgvc2lnbmF0dXJlIjoia1NvLzZQeUdkcnhnbFJFUFdVeUJRVEZxb3RMcmV4L040czRZRFV2d0N0aTl5NEpxWnkwaG92aUtCNkRtMDFCTEdKUU41b3hRdWdveXphSDVIcmVLRHc9PSJ9; atomic_session=eyJodHRwczovL2F0b21pY2RhdGEuZGV2L3Byb3BlcnRpZXMvYXV0aC9hZ2VudCI6Imh0dHBzOi8vYXRvbWljZGF0YS5kZXYvYWdlbnRzL1FtZnBSSUJuMkpZRWF0VDBNalNrTU5vQkp6c3R6MTlvcnduVDVvVDJyY1E9IiwiaHR0cHM6Ly9hdG9taWNkYXRhLmRldi9wcm9wZXJ0aWVzL2F1dGgvcmVxdWVzdGVkU3ViamVjdCI6Imh0dHBzOi8vc3RhZ2luZy5hdG9taWNkYXRhLmRldiIsImh0dHBzOi8vYXRvbWljZGF0YS5kZXYvcHJvcGVydGllcy9hdXRoL3B1YmxpY0tleSI6IlFtZnBSSUJuMkpZRWF0VDBNalNrTU5vQkp6c3R6MTlvcnduVDVvVDJyY1E9IiwiaHR0cHM6Ly9hdG9taWNkYXRhLmRldi9wcm9wZXJ0aWVzL2F1dGgvdGltZXN0YW1wIjoxNjc2MjgzMDQ2ODAzLCJodHRwczovL2F0b21pY2RhdGEuZGV2L3Byb3BlcnRpZXMvYXV0aC9zaWduYXR1cmUiOiIrVmQvc3VTV3U2Ykh4QXV3RUxBRjZ0a3NLNUFuVEpXL3g1L2RZRFFZUTdHS2Y3dXZPdUsycnYyaHVTb2c5SVMxOFppYXdpek8xcjJmVkU1aVdkTytCUT09In0%3D";

    let mut headermap = HeaderMap::new();
    headermap.insert(
        "Cookie".try_into().unwrap(),
        HeaderValue::from_str(cookie).unwrap(),
    );
    let subject = "https://staging.atomicdata.dev";
    let out = get_auth_from_cookie(&headermap, subject)
        .expect("Should not return err")
        .expect("Should contain cookie");

    assert_eq!(out.requested_subject, subject);
}
