//! OTel-standard attribute key constants for structured log/span/metric fields.
//!
//! No project-invented `myapp.*` keys.

pub const HTTP_REQUEST_METHOD: &str = "http.request.method";
pub const HTTP_ROUTE: &str = "http.route";
pub const HTTP_RESPONSE_STATUS_CODE: &str = "http.response.status_code";
pub const RPC_METHOD: &str = "rpc.method";
pub const RPC_SERVICE: &str = "rpc.service";
pub const ERROR_TYPE: &str = "error.type";
pub const ENDUSER_ID: &str = "enduser.id";
pub const SERVER_ADDRESS: &str = "server.address";
pub const SERVER_PORT: &str = "server.port";
pub const NETWORK_PROTOCOL_NAME: &str = "network.protocol.name";
pub const MESSAGING_SYSTEM: &str = "messaging.system";
pub const MESSAGING_DESTINATION_NAME: &str = "messaging.destination.name";
