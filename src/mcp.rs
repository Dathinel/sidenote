//! MCP wire adapter. Business operations live in core::agent_tools for other transports.
use crate::core::agent_tools;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, model::*, service::RequestContext};
use serde_json::{Value, json};

fn string(description: &str) -> Value {
    json!({"type":"string","description":description})
}
fn limit(max: usize, default: usize) -> Value {
    json!({"type":"integer","minimum":1,"maximum":max,"default":default})
}
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn definition(name: &str, title: &str, description: &str, input: Value, data: Value) -> Tool {
    serde_json::from_value(json!({
        "name":name,"title":title,"description":description,"inputSchema":input,
        "outputSchema":object(json!({"data":data,"warnings":{"type":"array","items":{"type":"string"}},
            "notes":{"type":"array","items":{"type":"string"}},"truncated":{"type":"boolean"}}),
            &["data","warnings","notes","truncated"]),
        "annotations":{"title":title,"readOnlyHint":name != "propose_memory","idempotentHint":true,
            "destructiveHint":false,"openWorldHint":false}
    })).expect("Static MCP tool definition")
}
pub fn tools() -> Vec<Tool> {
    let mut filters = json!({"from":string("Inclusive YYYY-MM-DD local calendar date or RFC 3339 instant with zone."),
        "to":string("Inclusive end date or RFC 3339 instant with zone. Unknown dates are excluded when filtering."),
        "project":string("Current project name or an alias."),"device":string("Local device name or peer folder label, case insensitive."),
        "limit":limit(50,20),"cursor":string("Opaque nextCursor from the previous page, with the same filters.")});
    let list_data = json!({"type":"object","properties":{
        "items":{"type":"array","maxItems":50,"items":{"type":"object"}},
        "nextCursor":{"type":["string","null"]},"hasMore":{"type":"boolean"}},"required":["items","nextCursor","hasMore"]});
    let list = definition(
        "list_meetings",
        "List meetings",
        "Browse meetings newest first, including read-only peers. Returns stable ids, title, zoned date, device, project and summary URI. Follow nextCursor for more.",
        object(filters.clone(), &[]),
        list_data.clone(),
    );
    filters["query"] = string(
        "Case-insensitive literal text across title, summary, decisions, current tasks, typed notes and the full transcript. Search first, then get_meeting.",
    );
    let config = crate::core::Config::load();
    vec![
        definition(
            "search_meetings",
            "Search meetings",
            "Find meetings by their content before reading details. Returns meeting metadata and a matching snippet, newest first. Covers all available local and peer meetings, not just recent ones.",
            object(filters, &["query"]),
            list_data.clone(),
        ),
        list,
        definition(
            "get_meeting",
            "Read a meeting",
            "Read selected parts of a meeting found by search/list. Defaults to summary, decisions, tasks and metadata. Fetch transcript only when needed, using nextOffset for subsequent UTF-8 byte pages. Read tasks for current merged tasks and ticks with the Tasks page cutoff. Summary todo is labelled as the original AI list. Typed notes and transcripts are opt-in UTF-8 byte pages. Feature switches are not access controls on meeting content.",
            object(
                json!({"id":string("Stable meeting id returned by search/list."),"parts":{"type":"array","minItems":1,"uniqueItems":true,
                "items":{"type":"string","enum":["summary","decisions","tasks","notes","transcript","participants","metadata"]}},
                "offset":{"type":"integer","minimum":0,"description":"Typed notes or transcript byte offset. Use nextOffset; never split a UTF-8 character."},
                "limit":limit(20480,20480)}),
                &["id"],
            ),
            json!({"type":"object","properties":{"id":{"type":"string"},"uri":{"type":"string"},
                    "summary":{"type":"object"},"decisions":{"type":"array"},"tasks":{"type":"array"},"participants":{"type":"array"},
                    "metadata":{"type":"object"},"notes":{"type":"object"},"transcript":{"type":"object"}},"required":["id","uri"]}),
        ),
        definition(
            "list_tasks",
            "List tasks",
            "Find To do, Waiting on, Review or Done work across meetings and manual tasks. Uses app edits, owner and team aliases, stable task ids, Mine flags, subtask ticks and the normal cleared-task cutoff. mine=true selects explicitly marked Mine; false includes everyone. Does not change tasks.",
            object(
                json!({"status":{"type":"string","enum":["todo","waiting","review","open","done","all"],"default":"open"},"mine":{"type":"boolean"},
                "project":string("Current project or alias."),"meeting_id":string("Filter to one meeting id."),"limit":limit(50,20),
                "cursor":string("Previous nextCursor with the same filters.")}),
                &[],
            ),
            list_data.clone(),
        ),
        definition(
            "list_teams", "List teams", "Discover defined teams, members and owner aliases. Available when Tasks is on. Follow nextCursor for more.",
            object(json!({"limit":limit(50,20),"cursor":string("Previous nextCursor.")}), &[]), list_data.clone(),
        ),
        definition(
            "get_memory",
            "Read memory",
            "Read the merged company memory, individual records with status and evidence, or proposals with their review status. Proposals are not accepted facts. Optional query is a literal case-insensitive substring. Follow nextCursor to continue.",
            object(
                json!({"scope":{"type":"string","enum":["company","records","proposals"]},"query":string("Literal substring to filter memory."),
                "limit":limit(50,20),"cursor":string("Previous nextCursor with the same scope and query.")}),
                &["scope"],
            ),
            list_data.clone(),
        ),
        definition(
            "list_projects",
            "List projects",
            "Discover current project names before filtering meetings or tasks. Resolves aliases and includes projects from peer meetings. Follow nextCursor to continue.",
            object(
                json!({"limit":limit(50,20),"cursor":string("Previous nextCursor.")}),
                &[],
            ),
            list_data,
        ),
        definition(
            "get_daily_check",
            "Read a daily check",
            "Read an existing merged daily report without running a check. Returns a UTF-8 byte page of the report JSON, including findings, evidence and proposals. Follow report.nextOffset; date is YYYY-MM-DD.",
            object(
                json!({"date":string("Report date YYYY-MM-DD."),"offset":{"type":"integer","minimum":0},"limit":limit(20480,20480)}),
                &["date"],
            ),
            object(
                json!({"date":{"type":"string"},"format":{"type":"string"},"report":{"type":"object"}}),
                &["date", "format", "report"],
            ),
        ),
        definition(
            "propose_memory",
            "Propose memory for review",
            "Queue a memory addition for the user to review in Sidenote. Never changes accepted records. Cite a source meeting when available; otherwise it is labelled an uncited MCP submission. Repeating the same content and source returns the same proposal, even after review.",
            object(
                json!({"text":{"type":"string","minLength":1,"maxLength":8000,"description":"Proposed claim, at most 8000 UTF-8 bytes."},
                "source_meeting_id":string("Optional meeting id obtained from search/list."),
                "reason":{"type":"string","minLength":1,"maxLength":2000,"description":"Why this should be remembered, at most 2000 UTF-8 bytes."}}),
                &["text", "reason"],
            ),
            object(
                json!({"id":{"type":"string"},"status":{"type":"string","enum":["pending","accepted","rejected"]},"reviewRequired":{"type":"boolean"}}),
                &["id", "status", "reviewRequired"],
            ),
        ),
    ].into_iter().filter(|tool| {
        crate::core::Feature::for_tool(&tool.name).is_none_or(|feature| {
            config.as_ref().is_ok_and(|c| c.features.enabled(feature))
        })
    }).collect()
}

#[derive(Clone)]
struct Meetings;
fn owner_backend() -> bool {
    cfg!(target_os = "linux") && crate::service::role() != crate::service::DeviceRole::Full
}
fn owner_request(method: &str, params: Value) -> Result<Value, String> {
    #[cfg(target_os = "linux")]
    {
        crate::sync_owner::local_request(&json!({"op":"mcp","method":method,"params":params}))
            .map_err(|_| "MCP_OWNER_UNAVAILABLE".into())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (method, params);
        Err("MCP_OWNER_UNAVAILABLE".into())
    }
}
fn catalog() -> Result<Vec<Tool>, String> {
    if owner_backend() {
        serde_json::from_value(owner_request("tools", json!({}))?)
            .map_err(|_| "MCP_OWNER_UNAVAILABLE".into())
    } else {
        Ok(tools())
    }
}
fn call(name: &str, arguments: Value) -> Result<Value, String> {
    if owner_backend() {
        owner_request("call", json!({"name":name,"arguments":arguments}))
    } else {
        agent_tools::call(name, arguments)
    }
}
fn resource(uri: &str) -> Result<String, String> {
    if owner_backend() {
        owner_request("resource", json!({"uri":uri}))?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| "MCP_OWNER_UNAVAILABLE".into())
    } else {
        agent_tools::meeting_resource(uri)
    }
}
impl ServerHandler for Meetings {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .build();
        ServerConfig::new(capabilities)
            .with_server_info(Implementation::new("sidenote", env!("CARGO_PKG_VERSION")))
            .with_instructions(agent_tools::INSTRUCTIONS)
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let tools = tokio::task::spawn_blocking(catalog)
            .await
            .map_err(|_| ErrorData::internal_error("MCP_OWNER_UNAVAILABLE", None))?
            .map_err(|_| ErrorData::internal_error("MCP_OWNER_UNAVAILABLE", None))?;
        Ok(ListToolsResult::with_all_items(tools))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        let result = tokio::task::spawn_blocking(move || call(&request.name, arguments)).await;
        let result = match result {
            Ok(Ok(value)) => {
                let summary = if let Some(items) = value["data"]["items"].as_array() {
                    format!(
                        "Returned {} items. More: {}. Truncated: {}.",
                        items.len(),
                        value["data"]["hasMore"],
                        value["truncated"]
                    )
                } else {
                    format!("Meeting data returned. Truncated: {}.", value["truncated"])
                };
                let mut result = CallToolResult::success(vec![ContentBlock::text(format!(
                    "{summary}\n{value}"
                ))]);
                if let Some(items) = value["data"]["items"].as_array() {
                    for item in items.iter().take(5) {
                        if let (Some(uri), Some(title)) =
                            (item["uri"].as_str(), item["title"].as_str())
                        {
                            let resource = Resource::new(uri, title);
                            result.content.push(ContentBlock::resource_link(resource));
                        }
                    }
                }
                result.structured_content = Some(value);
                result
            }
            Ok(Err(message)) => CallToolResult::error(vec![ContentBlock::text(message)]),
            Err(_) => CallToolResult::error(vec![ContentBlock::text(
                "Local data worker failed. Retry the request; inspect stderr if it persists.",
            )]),
        };
        Ok(result.into())
    }
    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("meeting://{id}", "Meeting summary"),
        ]))
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let lookup = uri.clone();
        let text = tokio::task::spawn_blocking(move || resource(&lookup))
            .await
            .map_err(|_| ErrorData::internal_error("Local resource reader failed; retry", None))?
            .map_err(|message| ErrorData::resource_not_found(message, None))?;
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, uri)]).into())
    }
}

/// Early CLI path. No Rune, Hex API, window guard, listener, or GPU initialization.
pub fn run() -> i32 {
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Could not start MCP runtime")
        .and_then(|runtime| {
            runtime.block_on(async {
                let service = Meetings
                    .serve(rmcp::transport::stdio())
                    .await
                    .map_err(|_| "MCP connection failed")?;
                service
                    .waiting()
                    .await
                    .map_err(|_| "MCP connection ended unexpectedly")?;
                Ok(())
            })
        });
    match result {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}
