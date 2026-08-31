use serde::Serialize;

use crate::verify::CheckResult;

#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    pub id: String,
    pub suite: String,
    pub description: String,
    pub passed: bool,
    pub expected_failure: bool,
    pub checks: Vec<CheckResult>,
    pub error: Option<String>,
    pub error_kind: Option<String>,
    pub executed_tcycles: usize,
    pub completed_early: bool,
    pub duration_ms: u64,
}

impl CaseResult {
    pub fn unexpected(&self) -> bool {
        self.passed == self.expected_failure
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Summary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub expected_failures: usize,
    pub unexpected: usize,
}

impl Summary {
    pub fn of(results: &[CaseResult]) -> Self {
        let passed = results.iter().filter(|result| result.passed).count();
        Self {
            total: results.len(),
            passed,
            failed: results.len() - passed,
            expected_failures: results
                .iter()
                .filter(|result| result.expected_failure)
                .count(),
            unexpected: results.iter().filter(|result| result.unexpected()).count(),
        }
    }
}

pub fn write_json(results: &[CaseResult]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "summary": Summary::of(results),
        "results": results,
    }))
    .expect("result serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(id: &str, passed: bool) -> CaseResult {
        CaseResult {
            id: id.into(),
            suite: "suite".into(),
            description: String::new(),
            passed,
            expected_failure: false,
            checks: Vec::new(),
            error: None,
            error_kind: None,
            executed_tcycles: 1,
            completed_early: false,
            duration_ms: 0,
        }
    }

    #[test]
    fn summarizes_and_serializes_results() {
        let results = [result("pass", true), result("fail", false)];
        let summary = Summary::of(&results);
        assert_eq!((summary.total, summary.passed, summary.failed), (2, 1, 1));
        let json = write_json(&results);
        assert!(json.contains("\"failed\": 1"));
        assert!(json.contains("\"id\": \"pass\""));
    }
}
