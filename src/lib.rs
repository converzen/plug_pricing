//! Pricing Plugin for MCP Server
//!
//! This plugin provides product pricing information from a PostgreSQL database.
//! It demonstrates the async MCP plugin API with automatic runtime management
//! and configuration support.


use mcp_plugin_api::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Pool, Postgres};


use tokio::runtime::Runtime;

use std::sync::OnceLock;
use tokio::sync::{mpsc, oneshot};


// ============================================================================
// Configuration
// ============================================================================

/// Plugin configuration structure
#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct PluginConfig {
    /// PostgreSQL database connection URL
    ///
    /// Example: "postgresql://user:password@localhost:5432/products"
    #[schemars(example = "example_database_url")]
    database_url: String,

    /// Maximum number of database connections in the pool
    #[schemars(range(min = 1, max = 100))]
    #[serde(default = "default_max_connections")]
    max_connections: u32,

    /// Connection timeout in seconds
    #[schemars(range(min = 1))]
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u64,
}

fn example_database_url() -> &'static str {
    "postgresql://user:password@localhost:5432/products"
}

fn default_max_connections() -> u32 {
    5
}

fn default_timeout_seconds() -> u64 {
    30
}

// Generate all configuration boilerplate with one macro!
declare_plugin_config!(PluginConfig);

// Generate configuration schema export
declare_config_schema!(PluginConfig);

// ============================================================================
// Database Setup
// ============================================================================


/// Product information from database
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
struct Product {
    id: i32,
    name: String,
    price: f64,
    description: Option<String>,
}

/// Initialize the database connection pool
async fn init_db_pool() -> Result<Pool<Postgres>, sqlx::Error> {
    let config = get_config();

    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .connect(&config.database_url)
        .await
}


// ============================================================================
// Plugin Initialization
// ============================================================================

static TX: OnceLock<mpsc:: UnboundedSender<Command>> = OnceLock::new();

struct McpRequest {
    payload: Value,
    responder: oneshot::Sender<Result<Value, String>>,
}

enum Command {
    GetProductPrice(McpRequest),
    SearchProducts(McpRequest),
}

enum InitResult {
    Success,
    Error(String),
}

fn ensure_runtime() -> &'static mpsc::UnboundedSender<Command> {
    TX.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<Command>();

        let (init_tx, init_rx) = oneshot::channel::<InitResult>();
        // Spawn a dedicated OS thread for our async world
        std::thread::spawn(move || {
            let rt = match Runtime::new() {
                Ok(rt) => rt,
                Err(err) => {
                    let _ = init_tx.send(InitResult::Error(err.to_string()));
                    return;
                }
            };
            rt.block_on(async {
                // async initialization her
                let pool = match init_db_pool().await {
                    Ok(pool) => pool,
                    Err(err) => {
                        let _ = init_tx.send(InitResult::Error(err.to_string()));
                        return
                    }
                };

                let _ = init_tx.send(InitResult::Success);

                while let Some(req) = rx.recv().await {
                    // Spawn a task for every request to allow internal parallelism
                    let pool_cpy = pool.clone();
                    tokio::spawn(async move {
                        match req {
                            Command::GetProductPrice(req) => {
                                let result = handle_get_product_price(&pool_cpy, &req.payload).await;
                                let _ = req.responder.send(result);
                            }
                            Command::SearchProducts(req) => {
                                let result = handle_search_products( &pool_cpy, &req.payload   ).await;
                                let _ = req.responder.send(result);
                            }
                        }
                    });
                }
            });
        });

        match init_rx.blocking_recv().unwrap() {
            InitResult::Success => tx,
            InitResult::Error(msg) => panic!("{}", msg),
        }
    })
}


/// Initialize plugin resources
///
/// This is called by the framework after configuration is set.
/// It validates the config and initializes the database connection.
fn init() -> Result<(), String> {
    // Create the async runtime
    let _tx = ensure_runtime();

    Ok(())
}

// Generate the plugin_init function
declare_plugin_init!(init);

// ============================================================================
// Tool Handlers - Now Async! 🚀
// ============================================================================

/// Handler for get_product_price tool
fn handle_get_product_price_sync(args: &Value) -> Result<Value, String> {
    let tx = ensure_runtime();
    let (resp_tx, resp_rx) = oneshot::channel();

    // 1. Offload work to the dedicated runtime
    tx.send(Command::GetProductPrice(McpRequest {
        payload: args.clone(),
        responder: resp_tx,
    })).ok();

    // 2. BLOCK the host thread using a light-weight executor
    // This does NOT try to start a new runtime, so it won't panic.
    futures::executor::block_on(resp_rx).map_err(|err| err.to_string())?
}

async fn handle_get_product_price(pool: &PgPool, args: &Value) -> Result<Value, String> {
    // Extract and validate product_id
    let product_id = args["product_id"]
        .as_i64()
        .ok_or("Missing or invalid product_id parameter")? as i32;

    // Execute async query directly - no manual runtime management!
    let product = sqlx::query_as::<_, Product>(
        "SELECT id, name, price, description FROM products WHERE id = $1",
    )
    .bind(product_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    match product {
        Some(p) => {
            // Return structured JSON data for programmatic clients
            Ok(utils::json_content(json!({
                "product": {
                    "id": p.id,
                    "name": p.name,
                    "price": p.price,
                    "description": p.description
                }
            })))
        }
        None => Err(format!("Product {product_id} not found",)),
    }
}

/// Handler for search_products tool
fn handle_search_products_sync( args: &Value) -> Result<Value, String> {
    let tx = ensure_runtime();
    let (resp_tx, resp_rx) = oneshot::channel();

    // 1. Offload work to the dedicated runtime
    tx.send(Command::SearchProducts(McpRequest {
        payload: args.clone(),
        responder: resp_tx,
    })).ok();

    // 2. BLOCK the host thread using a light-weight executor
    // This does NOT try to start a new runtime, so it won't panic.
    futures::executor::block_on(resp_rx).map_err(|err| err.to_string())?
}

async fn handle_search_products(pool: &PgPool, args: &Value) -> Result<Value, String> {
    // Extract and validate query
    let query = args["query"]
        .as_str()
        .ok_or("Missing or invalid query parameter")?;

    // Execute async query directly - no manual runtime management!
    let products = sqlx::query_as::<_, Product>(
        "SELECT id, name, price, description FROM products WHERE name ILIKE $1",
    )
    .bind(format!("%{query}%",))
    .fetch_all(pool)
    .await
    .map_err(|e| format!("Database error: {e}"))?;

    // Return structured JSON data for programmatic clients
    Ok(utils::json_content(json!({
        "products": products,
        "count": products.len()
    })))
}

// ============================================================================
// Plugin Declaration
// ============================================================================

// Declare tools using the standard macro
// Async handlers are wrapped with wrap_async_handler!
declare_tools! {
    tools: [
        Tool::builder("get_product_price", "Get the price of a product by ID")
            .param_i64("product_id", "The ID of the product", true)
            .handler(handle_get_product_price_sync),

        Tool::builder("search_products", "Search for products by name pattern")
            .param_string("query", "The search query (SQL LIKE pattern)", true)
            .handler(handle_search_products_sync),
    ]
}

// Declare the plugin with auto-generated functions, configuration, and init
declare_plugin! {
    list_tools: generated_list_tools,
    execute_tool: generated_execute_tool,
    free_string: utils::standard_free_string,
    configure: plugin_configure,
    init: plugin_init,
    get_config_schema: plugin_get_config_schema
}
