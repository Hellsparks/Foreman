//! The MCP tool catalogue: names, descriptions, typed arguments and safety class.
//!
//! Each tool maps onto today's REST API (see `tools.rs`); this file is only the
//! contract a model sees. Argument schemas are generated from [`Param`] lists, and
//! the same lists validate incoming arguments.

use serde::Serialize;
use serde_json::{json, Map, Value};
use ts_rs::TS;

/// How much a tool can change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export, rename = "McpToolClass")]
#[serde(rename_all = "snake_case")]
pub enum Class {
    /// Reads only. Allowed for every key by default.
    Read,
    /// Adds or edits records (vendors, filaments, maintenance log). Off by default.
    Write,
    /// Moves a printer, starts/cancels a print, deletes data or changes filament
    /// counts. Refused unless the key has destructive tools enabled.
    Destructive,
}

#[derive(Debug, Clone, Copy)]
pub enum Ty {
    Int,
    Num,
    Str,
    Bool,
    Enum(&'static [&'static str]),
    StrList,
    /// A boolean that must be `true` (a confirmation the user gave).
    Confirm,
}

#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub description: &'static str,
}

#[derive(Debug)]
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub class: Class,
    pub params: &'static [Param],
}

const fn req(name: &'static str, ty: Ty, description: &'static str) -> Param {
    Param {
        name,
        ty,
        required: true,
        description,
    }
}

const fn opt(name: &'static str, ty: Ty, description: &'static str) -> Param {
    Param {
        name,
        ty,
        required: false,
        description,
    }
}

const PRINTER_ID: Param = req(
    "printer_id",
    Ty::Int,
    "Printer id, from list_printers or get_fleet_status.",
);
const PLATE_CONFIRM: Param = req(
    "plate_confirm",
    Ty::Confirm,
    "Must be true. Set it only after the user has told you, in this conversation, that the \
     build plate of this printer is physically clear. Never assume it.",
);

pub const TOOLS: &[ToolDef] = &[
    // ----- Printers + fleet ---------------------------------------------------
    ToolDef {
        name: "get_fleet_status",
        title: "Fleet status",
        description: "Live state of every printer: idle/printing/paused/error/offline, file, \
                      progress, time left, temperatures, whether the plate is clear, plus open \
                      Attention items (things that need a person). Start here.",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "list_printers",
        title: "List printers",
        description: "Every configured printer with its id, name, firmware (moonraker, bambu, \
                      octoprint, …), tags, slicer profiles, hardened-nozzle flag and queue \
                      auto-start setting. Use get_fleet_status for live state.",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "get_printer_status",
        title: "Printer status",
        description: "Full live status of one printer: state, temperatures, progress, layer, \
                      fans, AMS trays and firmware error codes.",
        class: Class::Read,
        params: &[PRINTER_ID],
    },
    ToolDef {
        name: "get_printer_capabilities",
        title: "Printer capabilities",
        description: "What one printer reports about itself: bed size, extruder count, nozzle \
                      diameter, AMS slots, firmware version and the macro names run_macro accepts.",
        class: Class::Read,
        params: &[PRINTER_ID],
    },
    ToolDef {
        name: "list_attention",
        title: "Attention items",
        description: "Open Attention items: finished prints waiting for the plate to be cleared, \
                      errors, maintenance due, spool warnings.",
        class: Class::Read,
        params: &[],
    },
    // ----- Library ------------------------------------------------------------
    ToolDef {
        name: "list_library",
        title: "List Library folder",
        description: "Files and folders in one Library folder (G-code, 3MF, STL). Each file has \
                      an id used by get_library_file, check_file_compatibility and print_file.",
        class: Class::Read,
        params: &[opt(
            "folder",
            Ty::Str,
            "Folder path, e.g. \"Voron parts/2024\". Omit for the Library root.",
        )],
    },
    ToolDef {
        name: "get_library_file",
        title: "Library file details",
        description: "Everything Foreman parsed from one Library file: slicer, printer profile, \
                      nozzle, materials and grams per tool, print time, size, plates.",
        class: Class::Read,
        params: &[req(
            "file_id",
            Ty::Int,
            "Library file id, from list_library.",
        )],
    },
    ToolDef {
        name: "check_file_compatibility",
        title: "Check file against printers",
        description: "Can this file print on each printer? Per printer: verdict (ready, plate, \
                      busy, offline, too_big, nozzle, material, …) and a reason.",
        class: Class::Read,
        params: &[
            req("file_id", Ty::Int, "Library file id."),
            opt(
                "printer_id",
                Ty::Int,
                "Only report this printer. Omit for every printer.",
            ),
        ],
    },
    // ----- Send + queue ---------------------------------------------------------
    ToolDef {
        name: "get_queue",
        title: "Print queues",
        description: "The fleet queue and each printer's queue: waiting and active items in \
                      order, with item ids for remove_from_queue.",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "print_file",
        title: "Print a file now",
        description: "Upload a Library file to a printer and start printing it. Moves the \
                      printer. Runs the same compatibility check as the app first. Requires \
                      plate_confirm. Use dry_run to upload and check without starting.",
        class: Class::Destructive,
        params: &[
            req("file_id", Ty::Int, "Library file id."),
            PRINTER_ID,
            PLATE_CONFIRM,
            opt(
                "dry_run",
                Ty::Bool,
                "Upload and check only; do not start the print. Default false.",
            ),
            opt(
                "accept_manual_swaps",
                Ty::Bool,
                "The file pauses for hand filament swaps (M600); print it anyway. Default false.",
            ),
        ],
    },
    ToolDef {
        name: "add_to_queue",
        title: "Add to queue",
        description: "Add a Library file to a printer's queue, or to the fleet queue when \
                      printer_id is omitted. Queued items can start by themselves on a free \
                      printer with a clear plate.",
        class: Class::Destructive,
        params: &[
            req("file_id", Ty::Int, "Library file id."),
            opt(
                "printer_id",
                Ty::Int,
                "Printer whose queue to use. Omit for the fleet queue (any compatible printer).",
            ),
            opt(
                "accept_manual_swaps",
                Ty::Bool,
                "The file pauses for hand filament swaps (M600); queue it anyway.",
            ),
        ],
    },
    ToolDef {
        name: "remove_from_queue",
        title: "Remove from queue",
        description: "Remove a waiting item from a queue (an item that is already printing is \
                      cancelled in the queue only; use cancel_print to stop the printer).",
        class: Class::Destructive,
        params: &[req(
            "queue_item_id",
            Ty::Int,
            "Queue item id, from get_queue.",
        )],
    },
    ToolDef {
        name: "start_next_in_queue",
        title: "Start next queued print",
        description: "Start the next item in a printer's queue. Moves the printer. Requires \
                      plate_confirm.",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            PLATE_CONFIRM,
            opt(
                "dry_run",
                Ty::Bool,
                "Check and upload only; do not start. Default false.",
            ),
        ],
    },
    // ----- Printer control -------------------------------------------------------
    ToolDef {
        name: "pause_print",
        title: "Pause print",
        description: "Pause the current print on a printer.",
        class: Class::Destructive,
        params: &[PRINTER_ID],
    },
    ToolDef {
        name: "resume_print",
        title: "Resume print",
        description: "Resume a paused print.",
        class: Class::Destructive,
        params: &[PRINTER_ID],
    },
    ToolDef {
        name: "cancel_print",
        title: "Cancel print",
        description: "Cancel the current print. Cannot be undone. Requires confirm.",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            req(
                "confirm",
                Ty::Confirm,
                "Must be true: the user asked to cancel this printer's print.",
            ),
        ],
    },
    ToolDef {
        name: "set_temperature",
        title: "Set temperature",
        description: "Set the nozzle or bed target temperature in °C (0 turns the heater off).",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            req("heater", Ty::Enum(&["nozzle", "bed"]), "Which heater."),
            req("target", Ty::Num, "Target temperature in °C."),
        ],
    },
    ToolDef {
        name: "home_axes",
        title: "Home axes",
        description: "Home the printer's axes. Moves the printer.",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            opt(
                "axes",
                Ty::StrList,
                "Axes to home, e.g. [\"x\",\"y\"]. Omit to home all.",
            ),
        ],
    },
    ToolDef {
        name: "run_macro",
        title: "Run macro",
        description: "Run a firmware macro by name (see get_printer_capabilities for the list).",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            req("name", Ty::Str, "Macro name, e.g. \"PRINT_END\"."),
        ],
    },
    ToolDef {
        name: "send_gcode",
        title: "Send G-code",
        description: "Send one raw G-code line to a printer.",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            req("command", Ty::Str, "G-code, e.g. \"M106 S128\"."),
        ],
    },
    // ----- Jobs + history ----------------------------------------------------------
    ToolDef {
        name: "list_jobs",
        title: "Print history",
        description: "Finished, failed and cancelled prints, newest first, with printer, file, \
                      duration, filament used and outcome. All filters are optional.",
        class: Class::Read,
        params: &[
            opt("printer_id", Ty::Int, "Only this printer."),
            opt("from", Ty::Str, "Start date, YYYY-MM-DD."),
            opt("to", Ty::Str, "End date, YYYY-MM-DD."),
            opt("material", Ty::Str, "Material, e.g. \"PETG\"."),
            opt("spool_id", Ty::Int, "Only jobs that used this spool."),
            opt(
                "outcome",
                Ty::Enum(&["good", "failed", "cancelled", "unrated", "running"]),
                "Only this outcome: good (rated good), failed, cancelled, unrated (finished, \
                 not rated yet) or running.",
            ),
            opt("search", Ty::Str, "Text in the file name."),
            opt("limit", Ty::Int, "Max rows (default 50, max 500)."),
            opt("offset", Ty::Int, "Rows to skip, for paging."),
        ],
    },
    ToolDef {
        name: "get_job",
        title: "Job details",
        description: "One print job from the history.",
        class: Class::Read,
        params: &[req("job_id", Ty::Int, "Job id, from list_jobs.")],
    },
    ToolDef {
        name: "get_job_stats",
        title: "Print statistics",
        description: "Totals over a date range: prints, success rate, print hours, filament \
                      used, per printer and per material.",
        class: Class::Read,
        params: &[
            opt("from", Ty::Str, "Start date, YYYY-MM-DD."),
            opt("to", Ty::Str, "End date, YYYY-MM-DD."),
        ],
    },
    // ----- Spools, filaments, vendors ---------------------------------------------
    ToolDef {
        name: "list_spools",
        title: "List spools",
        description: "Spools with filament, colour, remaining grams, location and where each \
                      is loaded.",
        class: Class::Read,
        params: &[opt(
            "include_archived",
            Ty::Bool,
            "Include archived (used-up) spools. Default false.",
        )],
    },
    ToolDef {
        name: "get_spool",
        title: "Spool details",
        description: "One spool by id.",
        class: Class::Read,
        params: &[req("spool_id", Ty::Int, "Spool id, from list_spools.")],
    },
    ToolDef {
        name: "get_loaded_spools",
        title: "Loaded spools",
        description: "Which spool is loaded in each printer's tools / AMS slots.",
        class: Class::Read,
        params: &[opt(
            "printer_id",
            Ty::Int,
            "Only this printer. Omit for every printer.",
        )],
    },
    ToolDef {
        name: "load_spool",
        title: "Load spool",
        description: "Record that a spool is loaded in a printer's tool or AMS slot. Foreman \
                      checks material and diameter first; if it blocks, pass override_reason.",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            req("tool", Ty::Int, "Tool / slot index, 0-based."),
            req("spool_id", Ty::Int, "Spool id."),
            opt(
                "override_reason",
                Ty::Str,
                "Why to load despite a blocking warning (written to the audit log).",
            ),
        ],
    },
    ToolDef {
        name: "unload_spool",
        title: "Unload spool",
        description: "Record that a printer's tool or slot is now empty.",
        class: Class::Destructive,
        params: &[
            PRINTER_ID,
            req("tool", Ty::Int, "Tool / slot index, 0-based."),
        ],
    },
    ToolDef {
        name: "create_spool",
        title: "Add spools",
        description: "Add one or more identical spools of an existing filament to inventory.",
        class: Class::Destructive,
        params: &[
            req("filament_id", Ty::Int, "Filament id, from list_filaments."),
            opt(
                "initial_weight",
                Ty::Num,
                "Net filament weight when full, g. Default: the filament's.",
            ),
            opt(
                "spool_weight",
                Ty::Num,
                "Empty spool weight, g. Default: filament's, else vendor's.",
            ),
            opt("location", Ty::Str, "Storage location."),
            opt("lot_nr", Ty::Str, "Lot number."),
            opt("price", Ty::Num, "Price per spool."),
            opt("comment", Ty::Str, "Comment."),
            opt(
                "quantity",
                Ty::Int,
                "How many identical spools, 1–50. Default 1.",
            ),
        ],
    },
    ToolDef {
        name: "update_spool",
        title: "Edit spool",
        description: "Change a spool's location, lot, comment, price, or archive it. Only the \
                      fields you pass change.",
        class: Class::Destructive,
        params: &[
            req("spool_id", Ty::Int, "Spool id."),
            opt("location", Ty::Str, "Storage location."),
            opt("lot_nr", Ty::Str, "Lot number."),
            opt("comment", Ty::Str, "Comment."),
            opt("price", Ty::Num, "Price."),
            opt(
                "archived",
                Ty::Bool,
                "true archives the spool (used up); false restores it.",
            ),
        ],
    },
    ToolDef {
        name: "list_filaments",
        title: "List filaments",
        description: "Filament types (vendor, name, material, colours, temperatures, density) \
                      with spool counts.",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "get_filament",
        title: "Filament details",
        description: "One filament type by id.",
        class: Class::Read,
        params: &[req("filament_id", Ty::Int, "Filament id.")],
    },
    ToolDef {
        name: "create_filament",
        title: "Create filament",
        description: "Add a filament type. Give vendor_id or vendor_name (an unknown name \
                      creates the vendor).",
        class: Class::Write,
        params: FILAMENT_FORM,
    },
    ToolDef {
        name: "update_filament",
        title: "Edit filament",
        description: "Replace a filament type's details. Call get_filament first and pass every \
                      field you want to keep; omitted optional fields are cleared.",
        class: Class::Write,
        params: FILAMENT_UPDATE,
    },
    ToolDef {
        name: "list_vendors",
        title: "List vendors",
        description: "Filament vendors with empty-spool weight and filament count.",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "create_vendor",
        title: "Create vendor",
        description: "Add a filament vendor.",
        class: Class::Write,
        params: &[
            req("name", Ty::Str, "Vendor name, e.g. \"Prusament\"."),
            opt("empty_spool_weight", Ty::Num, "Empty spool weight, g."),
            opt("website", Ty::Str, "Website URL."),
            opt("comment", Ty::Str, "Comment."),
        ],
    },
    ToolDef {
        name: "update_vendor",
        title: "Edit vendor",
        description: "Change a vendor. Only the fields you pass change.",
        class: Class::Write,
        params: &[
            req("vendor_id", Ty::Int, "Vendor id, from list_vendors."),
            opt("name", Ty::Str, "New name."),
            opt("empty_spool_weight", Ty::Num, "Empty spool weight, g."),
            opt("website", Ty::Str, "Website URL."),
            opt("comment", Ty::Str, "Comment."),
        ],
    },
    // ----- Inventory ------------------------------------------------------------------
    ToolDef {
        name: "get_inventory",
        title: "Inventory",
        description: "Stock per tracked filament: spools on hand, grams left, minimum stock \
                      and which are low.",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "list_spool_locations",
        title: "Spool locations",
        description: "Storage locations and how many spools are in each.",
        class: Class::Read,
        params: &[],
    },
    // ----- Projects ----------------------------------------------------------------------
    ToolDef {
        name: "list_projects",
        title: "List projects",
        description: "Multi-plate projects with progress (plates done / total).",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "get_project",
        title: "Project details",
        description: "One project: its plates, copies, done counts and filament slots.",
        class: Class::Read,
        params: &[req("project_id", Ty::Int, "Project id.")],
    },
    ToolDef {
        name: "list_templates",
        title: "List project templates",
        description: "Reusable project templates (sets of Library files).",
        class: Class::Read,
        params: &[],
    },
    ToolDef {
        name: "print_project_remaining",
        title: "Queue a project's remaining plates",
        description: "Add every plate of a project that still needs printing to the fleet queue.",
        class: Class::Destructive,
        params: &[
            req("project_id", Ty::Int, "Project id."),
            opt(
                "accept_manual_swaps",
                Ty::Bool,
                "Some plates pause for hand filament swaps; queue them anyway.",
            ),
        ],
    },
    // ----- Maintenance ----------------------------------------------------------------------
    ToolDef {
        name: "get_maintenance",
        title: "Maintenance",
        description: "Maintenance tasks per printer with progress toward due (print hours, \
                      filament, days) and which are due.",
        class: Class::Read,
        params: &[opt("printer_id", Ty::Int, "Only this printer.")],
    },
    ToolDef {
        name: "get_maintenance_history",
        title: "Maintenance history",
        description: "When maintenance was done, by whom, with notes.",
        class: Class::Read,
        params: &[
            opt("printer_id", Ty::Int, "Only this printer."),
            opt("task_id", Ty::Int, "Only this task."),
        ],
    },
    ToolDef {
        name: "log_maintenance_done",
        title: "Log maintenance done",
        description: "Record that a maintenance task was done now (resets its counter).",
        class: Class::Write,
        params: &[
            req("task_id", Ty::Int, "Task id, from get_maintenance."),
            opt("notes", Ty::Str, "What was done."),
        ],
    },
    ToolDef {
        name: "snooze_maintenance",
        title: "Snooze maintenance",
        description: "Push a due maintenance task back by one step.",
        class: Class::Write,
        params: &[req("task_id", Ty::Int, "Task id.")],
    },
];

const FILAMENT_FIELDS_HEAD: [Param; 6] = [
    opt("vendor_id", Ty::Int, "Existing vendor id."),
    opt(
        "vendor_name",
        Ty::Str,
        "Vendor name; used when vendor_id is omitted (created if new).",
    ),
    req("name", Ty::Str, "Filament name, e.g. \"Galaxy Black\"."),
    req("material", Ty::Str, "Base material, e.g. \"PETG\"."),
    req(
        "colour_kind",
        Ty::Enum(&["single", "coaxial", "longitudinal"]),
        "single colour, coaxial (2–3 side by side) or longitudinal (gradient, 2–6).",
    ),
    req(
        "colours",
        Ty::StrList,
        "Colours as RRGGBB hex, e.g. [\"1A1A1A\"].",
    ),
];

const FILAMENT_FIELDS_TAIL: [Param; 9] = [
    opt(
        "modifiers",
        Ty::StrList,
        "Modifiers, e.g. [\"CF\"], [\"Silk\"], [\"Matte\"].",
    ),
    opt("extruder_temp", Ty::Int, "Nozzle temperature, °C."),
    opt("bed_temp", Ty::Int, "Bed temperature, °C."),
    opt(
        "density",
        Ty::Num,
        "Density, g/cm³. Default from the material.",
    ),
    opt("diameter", Ty::Num, "Diameter, mm. Default 1.75."),
    opt("price", Ty::Num, "Price per spool."),
    opt("url", Ty::Str, "Product URL."),
    opt("article_number", Ty::Str, "Article number / EAN."),
    opt("comment", Ty::Str, "Comment."),
];

const FILAMENT_FORM: &[Param] = &concat_params::<15>(&FILAMENT_FIELDS_HEAD, &FILAMENT_FIELDS_TAIL);
const FILAMENT_UPDATE: &[Param] = &concat_params::<16>(
    &[req("filament_id", Ty::Int, "Filament id.")],
    &concat_params::<15>(&FILAMENT_FIELDS_HEAD, &FILAMENT_FIELDS_TAIL),
);

const fn concat_params<const N: usize>(a: &[Param], b: &[Param]) -> [Param; N] {
    let mut out = [opt("", Ty::Str, ""); N];
    let mut i = 0;
    while i < a.len() {
        out[i] = a[i];
        i += 1;
    }
    let mut j = 0;
    while j < b.len() {
        out[i + j] = b[j];
        j += 1;
    }
    out
}

pub fn find(name: &str) -> Option<&'static ToolDef> {
    TOOLS.iter().find(|t| t.name == name)
}

fn ty_schema(ty: Ty, description: &str) -> Value {
    let mut s = match ty {
        Ty::Int => json!({ "type": "integer" }),
        Ty::Num => json!({ "type": "number" }),
        Ty::Str => json!({ "type": "string" }),
        Ty::Bool => json!({ "type": "boolean" }),
        Ty::Enum(v) => json!({ "type": "string", "enum": v }),
        Ty::StrList => json!({ "type": "array", "items": { "type": "string" } }),
        Ty::Confirm => json!({ "type": "boolean", "const": true }),
    };
    if let Some(o) = s.as_object_mut() {
        o.insert("description".into(), Value::String(description.to_owned()));
    }
    s
}

impl ToolDef {
    pub fn input_schema(&self) -> Value {
        let mut props = Map::new();
        let mut required = Vec::new();
        for p in self.params {
            props.insert(p.name.to_owned(), ty_schema(p.ty, p.description));
            if p.required {
                required.push(Value::String(p.name.to_owned()));
            }
        }
        json!({
            "type": "object",
            "properties": props,
            "required": required,
            "additionalProperties": false,
        })
    }

    /// The MCP `Tool` object for `tools/list`.
    pub fn to_mcp(&self) -> Value {
        let read = self.class == Class::Read;
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema(),
            "annotations": {
                "title": self.title,
                "readOnlyHint": read,
                "destructiveHint": self.class == Class::Destructive,
                "idempotentHint": read,
                "openWorldHint": false,
            },
        })
    }

    /// Check `args` against the parameter list. `Err` is a message for the model.
    pub fn validate(&self, args: &Map<String, Value>) -> Result<(), String> {
        for k in args.keys() {
            if !self.params.iter().any(|p| p.name == k) {
                let valid: Vec<&str> = self.params.iter().map(|p| p.name).collect();
                return Err(format!(
                    "unknown argument \"{k}\" for {}; valid arguments: {}",
                    self.name,
                    if valid.is_empty() {
                        "none".to_owned()
                    } else {
                        valid.join(", ")
                    }
                ));
            }
        }
        for p in self.params {
            let v = match args.get(p.name) {
                None | Some(Value::Null) => {
                    if p.required {
                        return Err(match p.ty {
                            Ty::Confirm if p.name == "plate_confirm" => {
                                "plate_confirm is required: ask the user to confirm the build \
                                 plate is clear, then call again with plate_confirm: true"
                                    .to_owned()
                            }
                            _ => format!("missing required argument \"{}\"", p.name),
                        });
                    }
                    continue;
                }
                Some(v) => v,
            };
            let ok = match p.ty {
                Ty::Int => v.as_i64().is_some(),
                Ty::Num => v.is_number(),
                Ty::Str => v.is_string(),
                Ty::Bool => v.is_boolean(),
                Ty::Enum(allowed) => v.as_str().is_some_and(|s| allowed.contains(&s)),
                Ty::StrList => v.as_array().is_some_and(|a| a.iter().all(Value::is_string)),
                Ty::Confirm => v == &Value::Bool(true),
            };
            if !ok {
                return Err(match p.ty {
                    Ty::Confirm if p.name == "plate_confirm" => {
                        "plate_confirm must be true: ask the user to confirm the build plate is \
                         clear before starting a print"
                            .to_owned()
                    }
                    Ty::Confirm => format!("\"{}\" must be true", p.name),
                    Ty::Enum(allowed) => {
                        format!("\"{}\" must be one of: {}", p.name, allowed.join(", "))
                    }
                    _ => format!("\"{}\" must be {}", p.name, ty_word(p.ty)),
                });
            }
        }
        Ok(())
    }
}

fn ty_word(ty: Ty) -> &'static str {
    match ty {
        Ty::Int => "an integer",
        Ty::Num => "a number",
        Ty::Str => "a string",
        Ty::Bool => "true or false",
        Ty::StrList => "a list of strings",
        Ty::Enum(_) | Ty::Confirm => "valid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_unique_and_valid() {
        let mut seen = std::collections::HashSet::new();
        for t in TOOLS {
            assert!(seen.insert(t.name), "duplicate {}", t.name);
            assert!(
                !t.name.is_empty()
                    && t.name.len() <= 128
                    && t.name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c)),
                "bad name {}",
                t.name
            );
            assert!(!t.description.is_empty());
            for p in t.params {
                assert!(!p.name.is_empty(), "{} has an empty param", t.name);
            }
        }
        assert!(TOOLS.len() >= 30, "port of the old ~30-tool set");
    }

    #[test]
    fn plate_confirm_required_on_print_starts() {
        for n in ["print_file", "start_next_in_queue"] {
            let t = find(n).expect("tool");
            assert_eq!(t.class, Class::Destructive);
            let mut a = Map::new();
            a.insert("printer_id".into(), json!(1));
            a.insert("file_id".into(), json!(1));
            let e = t.validate(&a).expect_err("needs plate_confirm");
            assert!(e.contains("plate"), "{e}");
            a.insert("plate_confirm".into(), json!(false));
            assert!(t.validate(&a).is_err());
            a.insert("plate_confirm".into(), json!(true));
            if n == "start_next_in_queue" {
                a.remove("file_id");
            }
            t.validate(&a).expect("ok");
        }
    }
}
