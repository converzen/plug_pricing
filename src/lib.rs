//! Pricing Plugin for MCP Server
//!
//! This plugin provides product pricing information from a PostgreSQL database.
//! It demonstrates the async MCP plugin API with automatic runtime management
//! and configuration support.

use log::debug;
use mcp_plugin_api::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::mpsc;

use std::time::Duration;
use tokio::runtime::Runtime;
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

// Database connection pool (initialized once, shared across threads)
static DB_POOL: once_cell::sync::OnceCell<Pool<Postgres>> = once_cell::sync::OnceCell::new();
static RUNTIME: once_cell::sync::OnceCell<Runtime> = once_cell::sync::OnceCell::new();

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

/// Get the database pool (panics if not initialized)
fn get_db_pool() -> &'static Pool<Postgres> {
    DB_POOL
        .get()
        .expect("Database pool not initialized - init() must be called first")
}

// ============================================================================
// Plugin Initialization
// ============================================================================

enum InitResult {
    Success,
    Error(String),
}

fn start_runtime_thread() -> Result<(), String> {
    // let config = get_config();
    let (tx, rx) = mpsc::channel();

    std::thread::spawn::<_, Result<(), String>>(move || {
        debug!("starting async runtime");

        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(err) => {
                let message = format!("failed to create runtime: {err}");
                tx.send(InitResult::Error(message))
                    .expect("failed to send message");
                return Ok(());
            }
        };

        RUNTIME.get_or_init(|| runtime);
        tx.send(InitResult::Success)
            .expect("failed to send message");

        loop {
            RUNTIME
                .get()
                .unwrap()
                .block_on(tokio::time::sleep(Duration::from_secs(604800)))
        }
    });

    let res = rx
        .recv()
        .map_err(|err| format!("Async thread receive error: {err}"))?;
    match res {
        InitResult::Success => Ok(()),
        InitResult::Error(message) => Err(message),
    }
}

/// Initialize plugin resources
///
/// This is called by the framework after configuration is set.
/// It validates the config and initializes the database connection.
fn init() -> Result<(), String> {
    start_runtime_thread()?;

    let config = get_config();
    // Validate configuration
    if config.max_connections == 0 {
        return Err("max_connections must be greater than 0".to_string());
    }

    // Create a runtime for initialization
    // Connect to database
    let pool = RUNTIME
        .get()
        .unwrap()
        .block_on(init_db_pool())
        .map_err(|e| format!("Failed to connect to database: {e}"))?;

    // Validate the connection works
    RUNTIME
        .get()
        .unwrap()
        .block_on(async { sqlx::query("SELECT 1").execute(&pool).await })
        .map_err(|e| format!("Database validation query failed: {e}"))?;

    // Store the pool
    DB_POOL
        .set(pool)
        .map_err(|_| "Database pool already initialized".to_string())?;

    Ok(())
}

// Generate the plugin_init function
declare_plugin_init!(init);

// ============================================================================
// Tool Handlers - Now Async! 🚀
// ============================================================================

/// Handler for get_product_price tool
fn handle_get_product_price_sync(args: &Value) -> Result<Value, String> {
    RUNTIME
        .get()
        .unwrap()
        .block_on(handle_get_product_price(args))
}

async fn handle_get_product_price(args: &Value) -> Result<Value, String> {
    // Extract and validate product_id
    let product_id = args["product_id"]
        .as_i64()
        .ok_or("Missing or invalid product_id parameter")? as i32;

    // Execute async query directly - no manual runtime management!
    let product = sqlx::query_as::<_, Product>(
        "SELECT id, name, price, description FROM products WHERE id = $1",
    )
    .bind(product_id)
    .fetch_optional(get_db_pool())
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
fn handle_search_products_sync(args: &Value) -> Result<Value, String> {
    RUNTIME
        .get()
        .unwrap()
        .block_on(handle_search_products(args))
}

async fn handle_search_products(args: &Value) -> Result<Value, String> {
    // Extract and validate query
    let query = args["query"]
        .as_str()
        .ok_or("Missing or invalid query parameter")?;

    // Execute async query directly - no manual runtime management!
    let products = sqlx::query_as::<_, Product>(
        "SELECT id, name, price, description FROM products WHERE name ILIKE $1",
    )
    .bind(format!("%{query}%",))
    .fetch_all(get_db_pool())
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
