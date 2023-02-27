//! Endpoints are experimental plugin-like objects, that allow for dynamic resources.
//! An endpoint is a resource that accepts one or more query parameters, and returns a resource that is probably calculated at runtime.
//! Examples of endpoints are versions for resources, or (pages for) collections.
//! See https://docs.atomicdata.dev/endpoints.html or https://atomicdata.dev/classes/Endpoint

use crate::{errors::AtomicResult, plugins, urls, Db, Resource, Storelike, Value};

/// The function that is called when a POST request matches the path
type HandleGet = fn(context: HandleGetContext) -> AtomicResult<Resource>;

/// The function that is called when a GET request matches the path
type HandlePost = fn(context: HandlePostContext) -> AtomicResult<Resource>;

/// Passed to an Endpoint GET request handler.
#[derive(Debug)]
pub struct HandleGetContext<'a> {
    /// The requested URL, including query parameters
    pub subject: url::Url,
    pub store: &'a Db,
    pub for_agent: Option<&'a str>,
}

/// Passed to an Endpoint POST request handler for.
#[derive(Debug)]
pub struct HandlePostContext<'a> {
    /// The requested URL, including query parameters
    pub subject: url::Url,
    pub store: &'a Db,
    pub for_agent: Option<&'a str>,
    pub body: Vec<u8>,
}
/// An API endpoint at some path which accepts requests and returns some Resource.
#[derive(Clone)]
pub struct Endpoint {
    /// The part behind the server domain, e.g. '/versions' or '/collections'. Include the slash.
    pub path: String,
    /// Called when a GET request matches the path.
    /// If none is given, the endpoint will return the basic Endpoint resource.
    pub handle: Option<HandleGet>,
    /// Called when a POST request matches the path.
    pub handle_post: Option<HandlePost>,
    /// The list of properties that can be passed to the Endpoint as Query parameters
    pub params: Vec<String>,
    pub description: String,
    pub shortname: String,
}

pub struct PostEndpoint {
    pub path: String,
    pub handle: Option<HandlePost>,
    pub params: Vec<String>,
    pub description: String,
    pub shortname: String,
}

impl Endpoint {
    /// Converts Endpoint to resource. Does not save it.
    pub fn to_resource(&self, store: &impl Storelike) -> AtomicResult<Resource> {
        let subject = format!("{}{}", store.get_server_url(), self.path);
        let mut resource = store.get_resource_new(&subject);
        resource.set_propval_string(urls::DESCRIPTION.into(), &self.description, store)?;
        resource.set_propval_string(urls::SHORTNAME.into(), &self.shortname, store)?;
        let is_a = [urls::ENDPOINT.to_string()].to_vec();
        resource.set_propval(urls::IS_A.into(), is_a.into(), store)?;
        let params_vec: Vec<String> = self.params.clone();
        resource.set_propval(
            urls::ENDPOINT_PARAMETERS.into(),
            Value::from(params_vec),
            store,
        )?;
        Ok(resource)
    }
}

pub fn default_endpoints() -> Vec<Endpoint> {
    vec![
        plugins::versioning::version_endpoint(),
        plugins::versioning::all_versions_endpoint(),
        plugins::path::path_endpoint(),
        plugins::search::search_endpoint(),
        plugins::files::upload_endpoint(),
        plugins::register::register_endpoint(),
        #[cfg(feature = "html")]
        plugins::bookmark::bookmark_endpoint(),
        plugins::importer::import_endpoint(),
    ]
}
