use crate::command::Address;
use std::path::PathBuf;

#[derive(Debug, Clone, Hash)]
pub enum Op {
    Inverse(Box<Op>),
    Append(AppendData),
    Delete(DeleteData),
    Edit(EditData),
}

#[derive(Debug, Clone, Hash, Default)]
pub struct AppendData {
    pub address: Option<Address>,
    pub current_line: usize,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Hash, Default)]
pub struct DeleteData {
    pub address: Option<Address>,
    pub lines_removed: Vec<String>,
    pub current_line: usize,
}

#[derive(Debug, Clone, Hash, Default)]
pub struct EditData {
    pub filename: PathBuf,
    pub current_line: usize,
    pub lines_removed: Vec<String>,
    pub clean_fingerprint: Option<u64>,
}

impl Op {
    pub fn inverse(&self) -> Op {
        match self {
            Op::Inverse(op) => *op.clone(),
            _ => Op::Inverse(Box::new(self.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_op() {
        let op = Op::Append(AppendData {
            address: None,
            current_line: 1,
            lines: Vec::new(),
        });
        let inv = op.inverse();
        assert!(match inv {
            Op::Inverse(bo) => matches!(*bo, Op::Append(_)),
            _ => false,
        });
    }

    #[test]
    fn inverse_inverse_op() {
        let op = Op::Append(AppendData {
            address: None,
            current_line: 1,
            lines: Vec::new(),
        });
        let inv = op.inverse();
        let inv_inv = inv.inverse();
        assert!(matches!(inv_inv, Op::Append(_)));
    }
}
