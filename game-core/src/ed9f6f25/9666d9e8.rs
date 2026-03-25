use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::error::{GameError, Result};

/// Value types in the interpreter
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    List(Vec<Value>),
    Null,
}

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            Value::Float(f) => Some(*f as i64),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Value::String(s) => s.clone(),
            Value::Int(n) => n.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::List(l) => format!("{:?}", l),
        }
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !s.is_empty(),
            Value::Null => false,
            Value::List(l) => !l.is_empty(),
        }
    }
}

/// Expression for evaluation
#[derive(Clone, Debug)]
pub enum Expr {
    Literal(Value),
    Var(String),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
}

#[derive(Clone, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Clone, Debug)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Instructions the interpreter can execute
#[derive(Clone, Debug)]
pub enum Instruction {
    /// Variable assignment: `x = 50`
    Set { var: String, expr: Expr },
    /// Label marker: `.loop`
    Label(String),
    /// Unconditional jump: `jump .loop`
    Jump(String),
    /// Conditional jump: `if x < 0 jump .invert`
    JumpIf { cond: Expr, label: String },
    /// Sleep in milliseconds: `sleep 16`
    Sleep(u64),
    /// Call external function: `call shuffle_deck`
    Call { func: String, args: Vec<Expr> },
    /// Comment (no-op)
    Comment(String),
    /// No operation
    Nop,
}

/// External function handler type
pub type ExternalFn =
    Box<dyn Fn(&[Value], &mut HashMap<String, Value>) -> Result<Value> + Send + Sync>;

/// Script interpreter for data files
pub struct Interpreter {
    /// Variable storage
    pub variables: HashMap<String, Value>,
    /// Label positions (label name -> instruction index)
    labels: HashMap<String, usize>,
    /// Program instructions
    instructions: Vec<Instruction>,
    /// Program counter
    pc: usize,
    /// External functions
    externals: HashMap<String, ExternalFn>,
    /// Whether execution is paused (for step-by-step)
    paused: bool,
    /// Sleep accumulator (milliseconds remaining)
    sleep_remaining: u64,
}

impl Interpreter {
    pub fn new() -> Self {
        Self {
            variables: HashMap::new(),
            labels: HashMap::new(),
            instructions: Vec::new(),
            pc: 0,
            externals: HashMap::new(),
            paused: false,
            sleep_remaining: 0,
        }
    }

    /// Load script from file
    pub fn load_file<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let content = fs::read_to_string(path)?;
        self.load_script(&content)
    }

    /// Load script from string
    pub fn load_script(&mut self, script: &str) -> Result<()> {
        self.instructions.clear();
        self.labels.clear();
        self.pc = 0;

        for line in script.lines() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            let instruction = self.parse_line(line)?;

            // Track label positions
            if let Instruction::Label(ref name) = instruction {
                self.labels.insert(name.clone(), self.instructions.len());
            }

            self.instructions.push(instruction);
        }

        Ok(())
    }

    /// Parse a single line into an instruction
    fn parse_line(&self, line: &str) -> Result<Instruction> {
        // Comment: `; this is a comment` or `// comment`
        if line.starts_with(';') || line.starts_with("//") {
            return Ok(Instruction::Comment(line.to_string()));
        }

        // Label: `.label_name`
        if line.starts_with('.') && !line.contains(' ') {
            return Ok(Instruction::Label(line.get(1..).unwrap_or("").to_string()));
        }

        // Jump: `jump .label`
        if line.starts_with("jump ") {
            let label = line.get(5..).unwrap_or("").trim();
            let label = if label.starts_with('.') {
                label.get(1..).unwrap_or("")
            } else {
                label
            };
            return Ok(Instruction::Jump(label.to_string()));
        }

        // Conditional jump: `if <cond> jump .label`
        if line.starts_with("if ") {
            return self.parse_conditional(line);
        }

        // Sleep: `sleep 16`
        if line.starts_with("sleep ") {
            let ms: u64 = line.get(6..).unwrap_or("").trim().parse()?;
            return Ok(Instruction::Sleep(ms));
        }

        // Call: `call func_name arg1 arg2`
        if line.starts_with("call ") {
            return self.parse_call(line.get(5..).unwrap_or(""));
        }

        // Assignment: `var = expr` or `var = value`
        if let Some(eq_pos) = line.find('=') {
            let var = line.get(..eq_pos).unwrap_or("").trim().to_string();
            let expr_str = line.get(eq_pos + 1..).unwrap_or("").trim();
            let expr = self.parse_expr(expr_str)?;
            return Ok(Instruction::Set { var, expr });
        }

        // Unknown - treat as comment
        Ok(Instruction::Comment(line.to_string()))
    }

    /// Parse conditional: `if x < 0 jump .label`
    fn parse_conditional(&self, line: &str) -> Result<Instruction> {
        // Format: if <expr> jump .label
        let rest = line.get(3..).unwrap_or(""); // Skip "if "

        if let Some(jump_pos) = rest.find(" jump ") {
            let cond_str = rest.get(..jump_pos).unwrap_or("").trim();
            let label = rest.get(jump_pos + 6..).unwrap_or("").trim();
            let label = if label.starts_with('.') {
                label.get(1..).unwrap_or("")
            } else {
                label
            };

            let cond = self.parse_condition(cond_str)?;
            return Ok(Instruction::JumpIf {
                cond,
                label: label.to_string(),
            });
        }

        Err(GameError::Script(format!("Invalid conditional: {}", line)))
    }

    /// Parse condition expression (e.g., `x < 0`, `pot > 0`)
    fn parse_condition(&self, cond: &str) -> Result<Expr> {
        // Simple comparison parsing
        let ops = [
            (">=", BinOp::Ge),
            ("<=", BinOp::Le),
            ("!=", BinOp::Ne),
            ("==", BinOp::Eq),
            (">", BinOp::Gt),
            ("<", BinOp::Lt),
        ];

        for (op_str, op) in ops {
            if let Some(pos) = cond.find(op_str) {
                let left = self.parse_expr(cond.get(..pos).unwrap_or("").trim())?;
                let right = self.parse_expr(cond.get(pos + op_str.len()..).unwrap_or("").trim())?;
                return Ok(Expr::BinOp(Box::new(left), op, Box::new(right)));
            }
        }

        // Single value as boolean
        self.parse_expr(cond)
    }

    /// Parse call: `func_name arg1 arg2`
    fn parse_call(&self, rest: &str) -> Result<Instruction> {
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.is_empty() {
            return Err(GameError::Script("Empty call".into()));
        }

        let func = match parts.first() {
            Some(s) => s.to_string(),
            None => return Err(GameError::Script("Empty call".into())),
        };
        let rest = parts.get(1..).unwrap_or(&[]);
        let args: Result<Vec<Expr>> = rest.iter().map(|s| self.parse_expr(s)).collect();

        Ok(Instruction::Call { func, args: args? })
    }

    /// Parse expression
    fn parse_expr(&self, expr: &str) -> Result<Expr> {
        let expr = expr.trim();

        // Check for binary operations (simple left-to-right parsing)
        let ops = [
            ("+", BinOp::Add),
            ("-", BinOp::Sub),
            ("*", BinOp::Mul),
            ("/", BinOp::Div),
            ("%", BinOp::Mod),
        ];

        for (op_str, op) in ops {
            // Find operator not at the start (to allow negative numbers)
            if let Some(pos) = expr.get(1..).unwrap_or("").find(op_str) {
                let pos = pos + 1;
                let left = self.parse_expr(expr.get(..pos).unwrap_or(""))?;
                let right = self.parse_expr(expr.get(pos + 1..).unwrap_or(""))?;
                return Ok(Expr::BinOp(Box::new(left), op, Box::new(right)));
            }
        }

        // Try parsing as literal
        if let Ok(n) = expr.parse::<i64>() {
            return Ok(Expr::Literal(Value::Int(n)));
        }
        if let Ok(f) = expr.parse::<f64>() {
            return Ok(Expr::Literal(Value::Float(f)));
        }
        if expr == "true" {
            return Ok(Expr::Literal(Value::Bool(true)));
        }
        if expr == "false" {
            return Ok(Expr::Literal(Value::Bool(false)));
        }
        if expr == "null" {
            return Ok(Expr::Literal(Value::Null));
        }
        if (expr.starts_with('"') && expr.ends_with('"'))
            || (expr.starts_with('\'') && expr.ends_with('\''))
        {
            return Ok(Expr::Literal(Value::String(
                expr.get(1..expr.len().saturating_sub(1))
                    .unwrap_or("")
                    .to_string(),
            )));
        }

        // Comma-separated list
        if expr.contains(',') {
            let items: Result<Vec<Value>> = expr
                .split(',')
                .map(|s| {
                    let e = self.parse_expr(s.trim())?;
                    self.eval_expr(&e)
                })
                .collect();
            return Ok(Expr::Literal(Value::List(items?)));
        }

        // Variable reference
        Ok(Expr::Var(expr.to_string()))
    }

    /// Evaluate expression with current variables
    fn eval_expr(&self, expr: &Expr) -> Result<Value> {
        match expr {
            Expr::Literal(v) => Ok(v.clone()),
            Expr::Var(name) => self
                .variables
                .get(name)
                .cloned()
                .ok_or_else(|| GameError::Script(format!("Undefined variable: {}", name))),
            Expr::BinOp(left, op, right) => {
                let l = self.eval_expr(left)?;
                let r = self.eval_expr(right)?;
                self.eval_binop(&l, op, &r)
            }
            Expr::UnaryOp(op, expr) => {
                let v = self.eval_expr(expr)?;
                match op {
                    UnaryOp::Neg => match v {
                        Value::Int(n) => Ok(Value::Int(-n)),
                        Value::Float(f) => Ok(Value::Float(-f)),
                        _ => Err(GameError::Script(format!("Cannot negate {:?}", v))),
                    },
                    UnaryOp::Not => Ok(Value::Bool(!v.as_bool())),
                }
            }
        }
    }

    /// Evaluate binary operation
    fn eval_binop(&self, left: &Value, op: &BinOp, right: &Value) -> Result<Value> {
        match op {
            BinOp::Add => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
                (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
                _ => Err(GameError::Script(format!(
                    "Cannot add {:?} and {:?}",
                    left, right
                ))),
            },
            BinOp::Sub => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
                _ => Err(GameError::Script(format!(
                    "Cannot subtract {:?} and {:?}",
                    left, right
                ))),
            },
            BinOp::Mul => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
                _ => Err(GameError::Script(format!(
                    "Cannot multiply {:?} and {:?}",
                    left, right
                ))),
            },
            BinOp::Div => match (left, right) {
                (Value::Int(a), Value::Int(b)) => match a.checked_div(*b) {
                    Some(result) => Ok(Value::Int(result)),
                    None => Err(GameError::Script("Division by zero or overflow".into())),
                },
                (Value::Float(a), Value::Float(b)) if *b != 0.0 => Ok(Value::Float(a / b)),
                (Value::Int(a), Value::Float(b)) if *b != 0.0 => Ok(Value::Float(*a as f64 / b)),
                (Value::Float(a), Value::Int(b)) if *b != 0 => Ok(Value::Float(a / *b as f64)),
                _ => Err(GameError::Script(
                    "Division by zero or invalid types".into(),
                )),
            },
            BinOp::Mod => match (left, right) {
                (Value::Int(a), Value::Int(b)) => match a.checked_rem(*b) {
                    Some(result) => Ok(Value::Int(result)),
                    None => Err(GameError::Script("Modulo by zero or overflow".into())),
                },
                _ => Err(GameError::Script("Modulo requires integers".into())),
            },
            BinOp::Eq => Ok(Value::Bool(left == right)),
            BinOp::Ne => Ok(Value::Bool(left != right)),
            BinOp::Lt => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) < *b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a < *b as f64)),
                _ => Err(GameError::Script(format!(
                    "Cannot compare {:?} < {:?}",
                    left, right
                ))),
            },
            BinOp::Le => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) <= *b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a <= *b as f64)),
                _ => Err(GameError::Script(format!(
                    "Cannot compare {:?} <= {:?}",
                    left, right
                ))),
            },
            BinOp::Gt => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) > *b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a > *b as f64)),
                _ => Err(GameError::Script(format!(
                    "Cannot compare {:?} > {:?}",
                    left, right
                ))),
            },
            BinOp::Ge => match (left, right) {
                (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
                (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
                (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) >= *b)),
                (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a >= *b as f64)),
                _ => Err(GameError::Script(format!(
                    "Cannot compare {:?} >= {:?}",
                    left, right
                ))),
            },
            BinOp::And => Ok(Value::Bool(left.as_bool() && right.as_bool())),
            BinOp::Or => Ok(Value::Bool(left.as_bool() || right.as_bool())),
        }
    }

    /// Register an external function
    pub fn register_external<F>(&mut self, name: &str, func: F)
    where
        F: Fn(&[Value], &mut HashMap<String, Value>) -> Result<Value> + Send + Sync + 'static,
    {
        self.externals.insert(name.to_string(), Box::new(func));
    }

    /// Get a variable value
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.variables.get(name)
    }

    /// Set a variable value
    pub fn set(&mut self, name: &str, value: Value) {
        self.variables.insert(name.to_string(), value);
    }

    /// Check if execution is complete
    pub fn is_done(&self) -> bool {
        self.pc >= self.instructions.len()
    }

    /// Execute one instruction (returns sleep time if Sleep instruction)
    pub fn step(&mut self) -> Result<Option<u64>> {
        if self.is_done() {
            return Ok(None);
        }

        let instruction = match self.instructions.get(self.pc) {
            Some(i) => i.clone(),
            None => return Ok(None),
        };
        self.pc += 1;

        match instruction {
            Instruction::Set { var, expr } => {
                let value = self.eval_expr(&expr)?;
                self.variables.insert(var, value);
            }
            Instruction::Label(_) => {
                // Labels are no-ops during execution
            }
            Instruction::Jump(label) => {
                if let Some(&target) = self.labels.get(&label) {
                    self.pc = target;
                } else {
                    return Err(GameError::Script(format!("Unknown label: {}", label)));
                }
            }
            Instruction::JumpIf { cond, label } => {
                let result = self.eval_expr(&cond)?;
                if result.as_bool() {
                    if let Some(&target) = self.labels.get(&label) {
                        self.pc = target;
                    } else {
                        return Err(GameError::Script(format!("Unknown label: {}", label)));
                    }
                }
            }
            Instruction::Sleep(ms) => {
                return Ok(Some(ms));
            }
            Instruction::Call { func, args } => {
                let arg_values: Result<Vec<Value>> =
                    args.iter().map(|e| self.eval_expr(e)).collect();
                let arg_values = arg_values?;

                if let Some(external) = self.externals.get(&func) {
                    let result = external(&arg_values, &mut self.variables)?;
                    self.variables.insert("_result".to_string(), result);
                } else {
                    // Unknown function - store call info for game to handle
                    self.variables
                        .insert("_pending_call".to_string(), Value::String(func));
                    self.variables
                        .insert("_pending_args".to_string(), Value::List(arg_values));
                }
            }
            Instruction::Comment(_) | Instruction::Nop => {
                // No-ops
            }
        }

        Ok(None)
    }

    /// Run until completion or sleep
    pub fn run_until_sleep(&mut self) -> Result<Option<u64>> {
        while !self.is_done() {
            if let Some(sleep_ms) = self.step()? {
                return Ok(Some(sleep_ms));
            }
        }
        Ok(None)
    }

    /// Reset execution to beginning
    pub fn reset(&mut self) {
        self.pc = 0;
        self.sleep_remaining = 0;
        self.paused = false;
    }

    /// Jump to a specific label
    pub fn jump_to(&mut self, label: &str) -> Result<()> {
        if let Some(&target) = self.labels.get(label) {
            self.pc = target;
            Ok(())
        } else {
            Err(GameError::Script(format!("Unknown label: {}", label)))
        }
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_assignment() {
        let mut interp = Interpreter::new();
        interp.load_script("x = 50\ny = 100").unwrap();
        interp.run_until_sleep().unwrap();

        assert_eq!(interp.get("x"), Some(&Value::Int(50)));
        assert_eq!(interp.get("y"), Some(&Value::Int(100)));
    }

    #[test]
    fn test_arithmetic() {
        let mut interp = Interpreter::new();
        interp.load_script("x = 10\ny = x + 5").unwrap();
        interp.run_until_sleep().unwrap();

        assert_eq!(interp.get("y"), Some(&Value::Int(15)));
    }

    #[test]
    fn test_jump() {
        let mut interp = Interpreter::new();
        interp
            .load_script(
                "x = 0
jump .skip
x = 100
.skip
y = 1",
            )
            .unwrap();
        interp.run_until_sleep().unwrap();

        assert_eq!(interp.get("x"), Some(&Value::Int(0)));
        assert_eq!(interp.get("y"), Some(&Value::Int(1)));
    }

    #[test]
    fn test_conditional() {
        let mut interp = Interpreter::new();
        interp
            .load_script(
                "x = 5
if x > 3 jump .big
result = 0
jump .end
.big
result = 1
.end",
            )
            .unwrap();
        interp.run_until_sleep().unwrap();

        assert_eq!(interp.get("result"), Some(&Value::Int(1)));
    }
}
