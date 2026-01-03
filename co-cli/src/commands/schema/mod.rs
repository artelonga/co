//! Schema CLI commands - `co schema`
//!
//! Manage feature type schemas.

use co::FeatureRegistry;
use co::feature::schema::PropertyKind;
use colored::Colorize;
use std::path::Path;

/// Schema subcommand action
pub enum SchemaAction {
    /// List all feature schemas
    List,
    /// Show a specific schema
    Show { name: String },
    /// Add a property to a schema
    AddProperty {
        feature: String,
        property: String,
        kind: String,
        required: bool,
    },
    /// Remove a property from a schema
    RemoveProperty {
        feature: String,
        property: String,
        force: bool,
    },
    /// Modify a property in a schema
    ModifyProperty {
        feature: String,
        property: String,
        kind: Option<String>,
        required: Option<bool>,
    },
}

/// Run schema command
pub fn run(action: SchemaAction) {
    match action {
        SchemaAction::List => list_schemas(),
        SchemaAction::Show { name } => show_schema(&name),
        SchemaAction::AddProperty {
            feature,
            property,
            kind,
            required,
        } => add_property(&feature, &property, &kind, required),
        SchemaAction::RemoveProperty {
            feature,
            property,
            force,
        } => remove_property(&feature, &property, force),
        SchemaAction::ModifyProperty {
            feature,
            property,
            kind,
            required,
        } => modify_property(&feature, &property, kind.as_deref(), required),
    }
}

/// List all feature schemas
fn list_schemas() {
    let current_dir = Path::new(".");
    let registry = FeatureRegistry::discover(current_dir);

    if registry.is_empty() {
        println!("{}", "No feature schemas found.".yellow());
        println!(
            "{}",
            "Create a feature directory with schema.yaml to define types.".dimmed()
        );
        return;
    }

    println!("{}", "Feature Schemas".bold());
    println!("{}", "═".repeat(50));

    for feature in registry.all() {
        let prop_count = feature
            .schema
            .as_ref()
            .map(|s| s.property_count())
            .unwrap_or(0);
        let version = feature.schema.as_ref().map(|s| s.version).unwrap_or(1);

        let source = if feature.is_system { "system" } else { "user" };

        println!(
            "  {} {} {} {}",
            feature.name.cyan().bold(),
            format!("v{}", version).dimmed(),
            format!("({} properties)", prop_count).dimmed(),
            format!("[{}]", source).dimmed()
        );
    }
}

/// Show a specific schema
fn show_schema(name: &str) {
    let current_dir = Path::new(".");
    let registry = FeatureRegistry::discover(current_dir);

    let feature = match registry.get(name) {
        Some(f) => f,
        None => {
            eprintln!("{} Feature '{}' not found", "error:".red().bold(), name);
            std::process::exit(1);
        }
    };

    let schema = match &feature.schema {
        Some(s) => s,
        None => {
            eprintln!("{} Feature '{}' has no schema", "error:".red().bold(), name);
            std::process::exit(1);
        }
    };

    println!("{} {}", "Feature:".bold(), schema.name.cyan());
    println!("{} {}", "Version:".bold(), schema.version);
    println!("{} {}", "Source:".bold(), feature.path.display());
    println!();

    if schema.properties.is_empty() {
        println!("{}", "No properties defined.".dimmed());
        return;
    }

    println!("{}", "Properties:".bold());
    println!("{}", "─".repeat(40));

    // Sort properties for consistent output
    let mut props: Vec<_> = schema.properties.iter().collect();
    props.sort_by_key(|(k, _)| *k);

    for (name, prop) in props {
        let kind_str = format!("{:?}", prop.kind).to_lowercase();
        let required_str = if prop.required {
            " (required)".yellow().to_string()
        } else {
            String::new()
        };

        println!("  {} : {}{}", name.green(), kind_str, required_str);
    }
}

/// Parse property kind from string
fn parse_kind(kind: &str) -> Option<PropertyKind> {
    match kind.to_lowercase().as_str() {
        "text" | "string" => Some(PropertyKind::Text),
        "number" | "num" | "int" => Some(PropertyKind::Number),
        "date" => Some(PropertyKind::Date),
        "list" | "array" => Some(PropertyKind::List),
        "link" | "url" => Some(PropertyKind::Link),
        "checkbox" | "bool" | "boolean" => Some(PropertyKind::Checkbox),
        _ => None,
    }
}

/// Add a property to a schema
fn add_property(feature_name: &str, property: &str, kind: &str, required: bool) {
    let current_dir = Path::new(".");
    let registry = FeatureRegistry::discover(current_dir);

    let feature = match registry.get(feature_name) {
        Some(f) => f,
        None => {
            eprintln!(
                "{} Feature '{}' not found",
                "error:".red().bold(),
                feature_name
            );
            std::process::exit(1);
        }
    };

    let mut schema = match &feature.schema {
        Some(s) => s.clone(),
        None => {
            eprintln!(
                "{} Feature '{}' has no schema",
                "error:".red().bold(),
                feature_name
            );
            std::process::exit(1);
        }
    };

    // Check if property already exists
    if schema.properties.contains_key(property) {
        eprintln!(
            "{} Property '{}' already exists in '{}'",
            "error:".red().bold(),
            property,
            feature_name
        );
        std::process::exit(1);
    }

    let prop_kind = match parse_kind(kind) {
        Some(k) => k,
        None => {
            eprintln!(
                "{} Unknown property kind: '{}'\n\nValid kinds: text, number, date, list, link, checkbox",
                "error:".red().bold(),
                kind
            );
            std::process::exit(1);
        }
    };

    schema.add_property(property.to_string(), prop_kind, required);

    if let Err(e) = schema.save(&feature.path) {
        eprintln!("{} Failed to save schema: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Added property '{}' to '{}' (v{})",
        "success:".green().bold(),
        property.cyan(),
        feature_name,
        schema.version
    );
}

/// Remove a property from a schema
fn remove_property(feature_name: &str, property: &str, force: bool) {
    let current_dir = Path::new(".");
    let registry = FeatureRegistry::discover(current_dir);

    let feature = match registry.get(feature_name) {
        Some(f) => f,
        None => {
            eprintln!(
                "{} Feature '{}' not found",
                "error:".red().bold(),
                feature_name
            );
            std::process::exit(1);
        }
    };

    let mut schema = match &feature.schema {
        Some(s) => s.clone(),
        None => {
            eprintln!(
                "{} Feature '{}' has no schema",
                "error:".red().bold(),
                feature_name
            );
            std::process::exit(1);
        }
    };

    // Check if property exists
    if !schema.properties.contains_key(property) {
        eprintln!(
            "{} Property '{}' not found in '{}'",
            "error:".red().bold(),
            property,
            feature_name
        );
        std::process::exit(1);
    }

    if !force {
        // Warn about potential files using this property
        println!(
            "{} Property '{}' may be used by existing files.",
            "warning:".yellow().bold(),
            property
        );
        println!("Use --force to confirm removal.");
        std::process::exit(1);
    }

    schema.remove_property(property);

    if let Err(e) = schema.save(&feature.path) {
        eprintln!("{} Failed to save schema: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Removed property '{}' from '{}' (v{})",
        "success:".green().bold(),
        property.cyan(),
        feature_name,
        schema.version
    );
}

/// Modify a property in a schema
fn modify_property(feature_name: &str, property: &str, kind: Option<&str>, required: Option<bool>) {
    let current_dir = Path::new(".");
    let registry = FeatureRegistry::discover(current_dir);

    let feature = match registry.get(feature_name) {
        Some(f) => f,
        None => {
            eprintln!(
                "{} Feature '{}' not found",
                "error:".red().bold(),
                feature_name
            );
            std::process::exit(1);
        }
    };

    let mut schema = match &feature.schema {
        Some(s) => s.clone(),
        None => {
            eprintln!(
                "{} Feature '{}' has no schema",
                "error:".red().bold(),
                feature_name
            );
            std::process::exit(1);
        }
    };

    // Check if property exists
    if !schema.properties.contains_key(property) {
        eprintln!(
            "{} Property '{}' not found in '{}'",
            "error:".red().bold(),
            property,
            feature_name
        );
        std::process::exit(1);
    }

    let prop_kind = if let Some(k) = kind {
        match parse_kind(k) {
            Some(pk) => Some(pk),
            None => {
                eprintln!(
                    "{} Unknown property kind: '{}'\n\nValid kinds: text, number, date, list, link, checkbox",
                    "error:".red().bold(),
                    k
                );
                std::process::exit(1);
            }
        }
    } else {
        None
    };

    if prop_kind.is_none() && required.is_none() {
        eprintln!(
            "{} Nothing to modify. Specify --kind or --required",
            "error:".red().bold()
        );
        std::process::exit(1);
    }

    schema.modify_property(property, prop_kind, required);

    if let Err(e) = schema.save(&feature.path) {
        eprintln!("{} Failed to save schema: {}", "error:".red().bold(), e);
        std::process::exit(1);
    }

    println!(
        "{} Modified property '{}' in '{}' (v{})",
        "success:".green().bold(),
        property.cyan(),
        feature_name,
        schema.version
    );
}
