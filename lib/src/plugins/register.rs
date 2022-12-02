//! Creates a new Drive and optionally also an Agent.

use serde::{Deserialize, Serialize};

use crate::{
    agents::Agent,
    email::{EmailAddress, MailAction, MailMessage},
    endpoints::{Endpoint, HandleGetContext},
    errors::AtomicResult,
    urls::{self},
    values::SubResource,
    Db, Resource, Storelike, Value,
};

pub fn register_endpoint() -> Endpoint {
    Endpoint {
      path: "/register".to_string(),
      params: [
        urls::NAME.to_string(),
        urls::EMAIL.to_string(),
      ].into(),
      description: "Allows new users to easily, in one request, make both an Agent and a Drive. This drive will be created at the subdomain of `name`.".to_string(),
      shortname: "register".to_string(),
      handle: Some(construct_register_redirect),
      handle_post: None,
  }
}

pub fn confirm_email_endpoint() -> Endpoint {
    Endpoint {
        path: "/confirmEmail".to_string(),
        params: [urls::TOKEN.to_string(), urls::INVITE_PUBKEY.to_string()].into(),
        description: "Confirm email address and set a key for your Agent.".to_string(),
        shortname: "confirm-email".to_string(),
        handle: Some(construct_confirm_email_redirect),
        handle_post: None,
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MailConfirmation {
    pub email: EmailAddress,
    pub name: String,
}

#[tracing::instrument()]
pub fn construct_register_redirect(context: HandleGetContext) -> AtomicResult<Resource> {
    let mut name_option = None;
    let mut email_option: Option<EmailAddress> = None;
    let store = context.store;
    for (k, v) in context.subject.query_pairs() {
        match k.as_ref() {
            "name" | urls::NAME => name_option = Some(v.to_string()),
            "email" => email_option = Some(EmailAddress::new(v.to_string())?),
            _ => {}
        }
    }
    // by default just return the Endpoint
    if name_option.is_none() && email_option.is_none() {
        return register_endpoint().to_resource(store);
    };

    let name = name_option.ok_or("No name provided")?;
    let email = email_option.ok_or("No email provided")?.check_used(store)?;

    // send the user an e-mail to confirm sign up
    let store_clone = store.clone();
    let confirmation_token_struct = MailConfirmation {
        email: email.clone(),
        name: name.clone(),
    };
    let token = crate::token::sign_claim(store, confirmation_token_struct)?;
    let mut confirm_url = store
        .get_server_url()
        .clone()
        .set_path("confirmEmail")
        .url();
    confirm_url.set_query(Some(&format!("token={}", token)));
    let message = MailMessage {
        to: email,
        subject: "Confirm your e-mail address".to_string(),
        body: format!("Welcome to Atomic Data, {}. Please confirm your e-mail address by clicking the link below", name),
        action: Some(MailAction {
            name: "Confirm e-mail address".to_string(),
            url: confirm_url.into()
        })
    };
    // async, because mails are slow
    tokio::spawn(async move {
        store_clone
            .send_email(message)
            .await
            .unwrap_or_else(|e| tracing::error!("Error sending email: {}", e));
    });

    // Here we probably want to return some sort of SuccesMessage page.
    // Not sure what that should be.
    let mut resource = Resource::new_generate_subject(store);
    resource.set_propval_string(urls::DESCRIPTION.into(), "success", store)?;

    // resource.set_propval(urls::, value, store)

    Ok(resource)
}

#[tracing::instrument()]
pub fn construct_confirm_email_redirect(context: HandleGetContext) -> AtomicResult<Resource> {
    let url = context.subject;
    let store = context.store;
    let mut token_opt: Option<String> = None;
    let mut pubkey_option = None;

    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "token" | urls::TOKEN => token_opt = Some(v.to_string()),
            "public-key" | urls::INVITE_PUBKEY => pubkey_option = Some(v.to_string()),
            _ => {}
        }
    }
    let token = if let Some(t) = token_opt {
        t
    } else {
        return confirm_email_endpoint().to_resource(store);
    };
    let pubkey = pubkey_option.ok_or("No public-key provided")?;

    // Parse and verify the JWT token
    let confirmation = crate::token::verify_claim::<MailConfirmation>(store, &token)?.custom;

    // Create the Agent if it doesn't exist yet.
    // Note: this happens before the drive is saved, which checks if the name is available.
    // We get new agents that just do nothing, but perhaps that's not a problem.
    let drive_creator_agent: String = {
        let mut new = Agent::new_from_public_key(store, &pubkey)?;
        new.name = Some(confirmation.name.clone());
        let net_agent_subject = new.subject.to_string();
        new.to_resource()?.save(store)?;
        net_agent_subject
    };

    // Create the new Drive
    let drive = crate::populate::create_drive(
        store,
        Some(&confirmation.name),
        &drive_creator_agent,
        false,
    )?;

    // Add the drive to the Agent's list of drives
    let mut agent = store.get_resource(&drive_creator_agent)?;
    agent.push_propval(
        urls::DRIVES,
        SubResource::Subject(drive.get_subject().into()),
        true,
    )?;
    // TODO: Make sure this only works if the server sets the email address.
    agent.set_propval(
        urls::EMAIL.into(),
        Value::String(confirmation.email.to_string()),
        store,
    )?;
    agent.save_locally(store)?;

    // Construct the Redirect Resource, which might provide the Client with a Subject for his Agent.
    let mut redirect = Resource::new_instance(urls::REDIRECT, store)?;
    redirect.set_propval_string(urls::DESTINATION.into(), drive.get_subject(), store)?;
    redirect.set_propval(
        urls::REDIRECT_AGENT.into(),
        crate::Value::AtomicUrl(drive_creator_agent),
        store,
    )?;
    Ok(redirect)
}
