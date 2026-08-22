use crate::bump::{BumpError, BumpType};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionMode {
    Semver,
    Calver,
}

impl VersionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semver => "semver",
            Self::Calver => "calver",
        }
    }
}

impl fmt::Display for VersionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuffixMode {
    #[serde(rename = "git_sha")]
    GitSha,
    Branch,
}

impl SuffixMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GitSha => "git_sha",
            Self::Branch => "branch",
        }
    }

    pub fn parse(value: &str) -> Result<Self, BumpError> {
        match value {
            "git_sha" => Ok(Self::GitSha),
            "branch" => Ok(Self::Branch),
            _ => Err(BumpError::LogicError(format!(
                "Invalid suffix mode: '{value}'. Expected 'git_sha' or 'branch'."
            ))),
        }
    }
}

impl fmt::Display for SuffixMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelPosition {
    BeforePrefix,
    AfterPrefix,
    BeforeBase,
    AfterBase,
    BeforePhase,
    AfterPhase,
}

impl LabelPosition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BeforePrefix => "before-prefix",
            Self::AfterPrefix => "after-prefix",
            Self::BeforeBase => "before-base",
            Self::AfterBase => "after-base",
            Self::BeforePhase => "before-phase",
            Self::AfterPhase => "after-phase",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timestamp {
    pub format: String,
    pub last: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Base {
    pub mode: VersionMode,
    pub delimiter: String,

    #[serde(alias = "year")]
    pub major: u32,

    #[serde(alias = "month")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minor: Option<u32>,

    #[serde(alias = "day")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phase {
    pub separator: String,
    pub name: String,
    pub delimiter: String,
    pub distance: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suffix {
    pub mode: SuffixMode,
    pub separator: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    pub position: LabelPosition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub prefix: String,
    pub base: Base,
    pub phase: Phase,
    pub suffix: Suffix,
    pub timestamp: Timestamp,
    pub label: Label,
}

/// Format `now` with a user-supplied chrono strftime string.
///
/// `chrono`'s `DelayedFormat` signals a bad format string by returning
/// `Err` from its `Display` impl, which makes `.to_string()` panic. Going
/// through `write!` lets us surface it as a normal error instead.
pub fn format_timestamp(now: DateTime<Utc>, format: &str) -> Result<String, BumpError> {
    let mut out = String::new();
    write!(out, "{}", now.format(format)).map_err(|_| {
        BumpError::LogicError(format!(
            "Invalid [timestamp].format '{format}': not a valid chrono strftime format"
        ))
    })?;
    Ok(out)
}

/// Increment a version component, reporting overflow instead of wrapping
/// (release builds) or panicking (debug builds).
fn increment(value: u32, component: &str) -> Result<u32, BumpError> {
    value.checked_add(1).ok_or_else(|| {
        BumpError::LogicError(format!(
            "Cannot bump {component}: value would overflow (max {})",
            u32::MAX
        ))
    })
}

/// `minor`/`patch` are optional in a bumpfile. Bumping one that was left out
/// is a mistake, not a no-op.
fn require(component: Option<u32>, key: &str) -> Result<u32, BumpError> {
    component.ok_or_else(|| {
        BumpError::LogicError(format!(
            "Cannot bump {key}: no '{key}' key in [base]. \
             Add '{key} = 0' to your bumpfile to track it."
        ))
    })
}

impl Version {
    fn right_mode(&self, expected_mode: VersionMode) -> Result<(), BumpError> {
        if self.base.mode == expected_mode {
            Ok(())
        } else {
            Err(BumpError::LogicError(format!(
                "Operation only valid for version.type = '{}'",
                expected_mode.as_str()
            )))
        }
    }

    fn clear_phase(&mut self) {
        self.phase.name = String::new();
        self.phase.distance = 0;
    }

    pub fn bump(&mut self, bump_type: &BumpType) -> Result<(), BumpError> {
        let now = chrono::Utc::now();
        match bump_type {
            BumpType::Major => {
                self.right_mode(VersionMode::Semver)?;
                self.base.major = increment(self.base.major, "major")?;
                self.base.minor = self.base.minor.map(|_| 0);
                self.base.patch = self.base.patch.map(|_| 0);
                self.clear_phase();
            }
            BumpType::Minor => {
                self.right_mode(VersionMode::Semver)?;
                let minor = require(self.base.minor, "minor")?;
                self.base.minor = Some(increment(minor, "minor")?);
                self.base.patch = self.base.patch.map(|_| 0);
                self.clear_phase();
            }
            BumpType::Patch => {
                self.right_mode(VersionMode::Semver)?;
                let patch = require(self.base.patch, "patch")?;
                self.base.patch = Some(increment(patch, "patch")?);
                self.clear_phase();
            }
            BumpType::Phase(cli_phase_name) => {
                if cli_phase_name != &self.phase.name && *cli_phase_name != "__increment__" {
                    self.phase.name.clone_from(cli_phase_name);
                    self.phase.distance = 1;
                } else {
                    self.phase.distance = increment(self.phase.distance, "phase distance")?;
                }
            }
            BumpType::Calendar => {
                self.right_mode(VersionMode::Calver)?;
                if now.year().cast_unsigned() == self.base.major
                    && now.month() == self.base.minor.unwrap_or(0)
                    && now.day() == self.base.patch.unwrap_or(0)
                {
                    self.phase.distance = increment(self.phase.distance, "phase distance")?;
                } else {
                    self.base.major = now.year().cast_unsigned();
                    self.base.minor = self.base.minor.map(|_| now.month());
                    self.base.patch = self.base.patch.map(|_| now.day());
                }
            }
        }
        self.timestamp.last = format_timestamp(now, &self.timestamp.format)?;
        Ok(())
    }
}
