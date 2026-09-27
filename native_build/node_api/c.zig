//! Raw declarations from consumer-selected npm Node-API headers.

const options = @import("node_api_options");

pub const c = @cImport({
    @cDefine("NAPI_VERSION", options.napi_version_text);
    @cDefine("BUILDING_NODE_EXTENSION", "1");
    @cInclude("node_api.h");
});

pub const selected_version_text = options.napi_version_text;
