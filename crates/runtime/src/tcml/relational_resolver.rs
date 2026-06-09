#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileReadRelationalDefault {
    OffsetForLimitedRead { offset: usize },
    LimitForOffsetRead { limit: usize },
}

impl FileReadRelationalDefault {
    pub fn issue_path(self) -> &'static str {
        match self {
            FileReadRelationalDefault::OffsetForLimitedRead { .. } => "offset",
            FileReadRelationalDefault::LimitForOffsetRead { .. } => "limit",
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            FileReadRelationalDefault::OffsetForLimitedRead { .. } => "limit_without_offset",
            FileReadRelationalDefault::LimitForOffsetRead { .. } => "offset_without_limit",
        }
    }

    pub fn repair_rule(self) -> &'static str {
        match self {
            FileReadRelationalDefault::OffsetForLimitedRead { .. } => {
                "default_offset_for_limited_read"
            }
            FileReadRelationalDefault::LimitForOffsetRead { .. } => "default_limit_for_offset_read",
        }
    }

    pub fn value(self) -> usize {
        match self {
            FileReadRelationalDefault::OffsetForLimitedRead { offset } => offset,
            FileReadRelationalDefault::LimitForOffsetRead { limit } => limit,
        }
    }
}

pub fn default_file_read_offset_for_limit() -> usize {
    0
}

pub fn default_file_read_limit_for_offset() -> usize {
    2000
}

pub fn file_read_relational_default(
    limit: Option<usize>,
    offset: Option<usize>,
) -> Option<FileReadRelationalDefault> {
    if limit.is_some() && offset.is_none() {
        return Some(FileReadRelationalDefault::OffsetForLimitedRead {
            offset: default_file_read_offset_for_limit(),
        });
    }
    if offset.is_some() && limit.is_none() {
        return Some(FileReadRelationalDefault::LimitForOffsetRead {
            limit: default_file_read_limit_for_offset(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_file_read_limit_offset_defaults() {
        let offset = file_read_relational_default(Some(10), None).unwrap();
        assert_eq!(offset.issue_path(), "offset");
        assert_eq!(offset.value(), 0);
        assert_eq!(offset.reason(), "limit_without_offset");

        let limit = file_read_relational_default(None, Some(5)).unwrap();
        assert_eq!(limit.issue_path(), "limit");
        assert_eq!(limit.value(), 2000);
        assert_eq!(limit.reason(), "offset_without_limit");

        assert_eq!(file_read_relational_default(Some(10), Some(5)), None);
    }
}
