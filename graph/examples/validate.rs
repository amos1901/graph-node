/// Validate subgraph schemas by parsing them into `InputSchema` and making
/// sure that they are valid
///
/// The input files must be in a particular format; that can be generated by
/// running this script against graph-node shard(s). Before running it,
/// change the `dbs` variable to list all databases against which it should
/// run.
///
/// ```
/// #! /bin/bash
///
/// read -r -d '' query <<EOF
/// \copy (select to_jsonb(a.*) from (select id, schema from subgraphs.subgraph_manifest) a) to '%s'
/// EOF
///
/// dbs="shard1 shard2 .."
///
/// dir=/var/tmp/schemas
/// mkdir -p $dir
///
/// for db in $dbs
/// do
///     echo "Dump $db"
///     q=$(printf "$query" "$dir/$db.json")
///     psql -qXt service=$db -c "$q"
///     sed -r -i -e 's/\\\\/\\/g' "$dir/$db.json"
/// done
///
/// ```
use clap::Parser;

use graph::data::graphql::ext::DirectiveFinder;
use graph::data::graphql::DirectiveExt;
use graph::data::graphql::DocumentExt;
use graph::data::subgraph::SPEC_VERSION_1_1_0;
use graph::prelude::s;
use graph::prelude::DeploymentHash;
use graph::schema::InputSchema;
use graphql_parser::parse_schema;
use serde::Deserialize;
use std::env;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::process::exit;

pub fn usage(msg: &str) -> ! {
    println!("{}", msg);
    println!("usage: validate schema.graphql ...");
    println!("\nValidate subgraph schemas");
    std::process::exit(1);
}

pub fn ensure<T, E: std::fmt::Display>(res: Result<T, E>, msg: &str) -> T {
    match res {
        Ok(ok) => ok,
        Err(err) => {
            eprintln!("{}:\n    {}", msg, err);
            exit(1)
        }
    }
}

fn subgraph_id(schema: &s::Document) -> DeploymentHash {
    let id = schema
        .get_object_type_definitions()
        .first()
        .and_then(|obj_type| obj_type.find_directive("subgraphId"))
        .and_then(|dir| dir.argument("id"))
        .and_then(|arg| match arg {
            s::Value::String(s) => Some(s.to_owned()),
            _ => None,
        })
        .unwrap_or("unknown".to_string());
    DeploymentHash::new(id).expect("subgraph id is not a valid deployment hash")
}

#[derive(Deserialize)]
struct Entry {
    id: i32,
    schema: String,
}

#[derive(Parser)]
#[clap(
    name = "validate",
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = "Validate subgraph schemas"
)]
struct Opts {
    /// Validate a batch of schemas in bulk. When this is set, the input
    /// files must be JSONL files where each line has an `id` and a `schema`
    #[clap(short, long)]
    batch: bool,
    #[clap(long)]
    api: bool,
    /// Subgraph schemas to validate
    #[clap(required = true)]
    schemas: Vec<String>,
}

fn parse(raw: &str, name: &str, api: bool) {
    let schema = ensure(
        parse_schema(raw).map(|v| v.into_static()),
        &format!("Failed to parse schema sgd{}", name),
    );
    let id = subgraph_id(&schema);
    let input_schema = match InputSchema::parse(&SPEC_VERSION_1_1_0, raw, id.clone()) {
        Ok(schema) => schema,
        Err(e) => {
            println!("InputSchema: {}[{}]: {}", name, id, e);
            return;
        }
    };
    if api {
        let _api_schema = match input_schema.api_schema() {
            Ok(schema) => schema,
            Err(e) => {
                println!("ApiSchema: {}[{}]: {}", name, id, e);
                return;
            }
        };
    }
    println!("Schema {}[{}]: OK", name, id);
}

pub fn main() {
    // Allow fulltext search in schemas
    std::env::set_var("GRAPH_ALLOW_NON_DETERMINISTIC_FULLTEXT_SEARCH", "true");

    let opt = Opts::parse();

    if opt.batch {
        for schema in &opt.schemas {
            println!("Validating schemas from {schema}");
            let file = File::open(schema).expect("file exists");
            let rdr = BufReader::new(file);
            for line in rdr.lines() {
                let line = line.expect("invalid line").replace("\\\\", "\\");
                let entry = serde_json::from_str::<Entry>(&line).expect("line is valid json");

                let raw = &entry.schema;
                let name = format!("sgd{}", entry.id);
                parse(raw, &name, opt.api);
            }
        }
    } else {
        for schema in &opt.schemas {
            println!("Validating schema from {schema}");
            let raw = std::fs::read_to_string(schema).expect("file exists");
            parse(&raw, schema, opt.api);
        }
    }
}
