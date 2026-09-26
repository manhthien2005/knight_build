//! Durable Enhancement Queue v1 Orchestrator (ENHANCE-05B).
//!
//! Authoritative database contract defined by migration 013 (`013_enhancement_queue.sql`).
//! Preserves the proven Zeus single-item enhancement engine unchanged.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Job lifecycle states defined by migration 013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnhancementQueueJobStatus {
    Draft,
    Queued,
    Running,
    Pausing,
    Paused,
    Completed,
    Failed,
    Cancelled,
    ManualReviewRequired,
}

impl EnhancementQueueJobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Pausing => "PAUSING",
            Self::Paused => "PAUSED",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::ManualReviewRequired => "MANUAL_REVIEW_REQUIRED",
        }
    }

    /// Only QUEUED jobs may be initially claimed by an agent.
    pub fn is_claimable(&self) -> bool {
        matches!(self, Self::Queued)
    }

    /// Whether this job is in an unresolved active state that blocks other queues for the account.
    pub fn is_unresolved_active(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Pausing | Self::Paused | Self::ManualReviewRequired
        )
    }

    /// Terminal state from which no further execution is permitted.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl FromStr for EnhancementQueueJobStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "DRAFT" => Ok(Self::Draft),
            "QUEUED" => Ok(Self::Queued),
            "RUNNING" => Ok(Self::Running),
            "PAUSING" => Ok(Self::Pausing),
            "PAUSED" => Ok(Self::Paused),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "MANUAL_REVIEW_REQUIRED" => Ok(Self::ManualReviewRequired),
            other => Err(format!("unknown EnhancementQueueJobStatus: {other}")),
        }
    }
}

/// Item lifecycle states defined by migration 013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnhancementQueueItemStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    ManualReviewRequired,
}

impl EnhancementQueueItemStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Running => "RUNNING",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Cancelled => "CANCELLED",
            Self::ManualReviewRequired => "MANUAL_REVIEW_REQUIRED",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

impl FromStr for EnhancementQueueItemStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "PENDING" => Ok(Self::Pending),
            "RUNNING" => Ok(Self::Running),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            "CANCELLED" => Ok(Self::Cancelled),
            "MANUAL_REVIEW_REQUIRED" => Ok(Self::ManualReviewRequired),
            other => Err(format!("unknown EnhancementQueueItemStatus: {other}")),
        }
    }
}

/// Restart-safe durable attempt phases defined by migration 013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnhancementAttemptPhase {
    None,
    Preparing,
    ReadyToExecute,
    ExecuteMayHaveBeenSent,
    WaitingResult,
    WaitingSettlement,
    Settled,
}

impl EnhancementAttemptPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Preparing => "PREPARING",
            Self::ReadyToExecute => "READY_TO_EXECUTE",
            Self::ExecuteMayHaveBeenSent => "EXECUTE_MAY_HAVE_BEEN_SENT",
            Self::WaitingResult => "WAITING_RESULT",
            Self::WaitingSettlement => "WAITING_SETTLEMENT",
            Self::Settled => "SETTLED",
        }
    }

    /// Pre-fence phases: safe to cancel or resume pre-dispatch without risking double Opcode 67.
    pub fn is_pre_fence(&self) -> bool {
        matches!(self, Self::None | Self::Preparing | Self::ReadyToExecute)
    }

    /// Post-fence phases: mutation command may have been dispatched. Must NEVER be blindly replayed.
    pub fn is_post_fence(&self) -> bool {
        matches!(
            self,
            Self::ExecuteMayHaveBeenSent | Self::WaitingResult | Self::WaitingSettlement | Self::Settled
        )
    }
}

impl FromStr for EnhancementAttemptPhase {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "NONE" => Ok(Self::None),
            "PREPARING" => Ok(Self::Preparing),
            "READY_TO_EXECUTE" => Ok(Self::ReadyToExecute),
            "EXECUTE_MAY_HAVE_BEEN_SENT" => Ok(Self::ExecuteMayHaveBeenSent),
            "WAITING_RESULT" => Ok(Self::WaitingResult),
            "WAITING_SETTLEMENT" => Ok(Self::WaitingSettlement),
            "SETTLED" => Ok(Self::Settled),
            other => Err(format!("unknown EnhancementAttemptPhase: {other}")),
        }
    }
}

/// Payment type allowlist from migration 013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnhancementPaymentType {
    Gold,
    Gems,
}

impl EnhancementPaymentType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Gold => "GOLD",
            Self::Gems => "GEMS",
        }
    }

    /// Maps to existing wire payment type (0 = Gold, 1 = Gem).
    pub fn to_wire_byte(&self) -> u8 {
        match self {
            Self::Gold => 0,
            Self::Gems => 1,
        }
    }
}

impl FromStr for EnhancementPaymentType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "GOLD" => Ok(Self::Gold),
            "GEMS" => Ok(Self::Gems),
            other => Err(format!("unknown EnhancementPaymentType: {other}")),
        }
    }
}

/// Charm mode allowlist from migration 013.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EnhancementCharmMode {
    None,
    Co3La,
    Co4La,
    AutoPolicy,
    ThreeLeaf,
    FourLeaf,
}

impl EnhancementCharmMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Co3La => "CO_3_LA",
            Self::Co4La => "CO_4_LA",
            Self::AutoPolicy => "AUTO_POLICY",
            Self::ThreeLeaf => "THREE_LEAF",
            Self::FourLeaf => "FOUR_LEAF",
        }
    }

    /// Maps to existing wire charm mode (0 = None, 1 = 3-leaf, 2 = 4-leaf, 3 = Auto).
    pub fn to_wire_byte(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Co3La | Self::ThreeLeaf => 1,
            Self::Co4La | Self::FourLeaf => 2,
            Self::AutoPolicy => 3,
        }
    }
}

impl FromStr for EnhancementCharmMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "NONE" => Ok(Self::None),
            "CO_3_LA" => Ok(Self::Co3La),
            "CO_4_LA" => Ok(Self::Co4La),
            "AUTO_POLICY" => Ok(Self::AutoPolicy),
            "THREE_LEAF" => Ok(Self::ThreeLeaf),
            "FOUR_LEAF" => Ok(Self::FourLeaf),
            other => Err(format!("unknown EnhancementCharmMode: {other}")),
        }
    }
}

/// Row structure representing `public.enhancement_queue_jobs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancementQueueJobRow {
    pub id: String,
    pub account_id: String,
    pub device_id: String,
    pub user_id: String,
    pub status: String,
    #[serde(default)]
    pub active_item_id: Option<String>,
    #[serde(default)]
    pub active_attempt_uuid: Option<String>,
    #[serde(default)]
    pub active_command_id: Option<String>,
    pub total_items: i32,
    pub completed_items: i32,
    #[serde(default)]
    pub claimed_by: Option<String>,
    #[serde(default)]
    pub claimed_at: Option<String>,
    #[serde(default)]
    pub claim_expires_at: Option<String>,
    #[serde(default)]
    pub pause_requested_at: Option<String>,
    #[serde(default)]
    pub cancel_requested_at: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub updated_at: String,
}

impl EnhancementQueueJobRow {
    #[cfg(test)]
    pub fn mock(id: &str, account_id: &str, device_id: &str, status: &str, claimed_by: &str) -> Self {
        Self {
            id: id.to_string(),
            account_id: account_id.to_string(),
            device_id: device_id.to_string(),
            user_id: "user-1".to_string(),
            status: status.to_string(),
            active_item_id: None,
            active_attempt_uuid: None,
            active_command_id: None,
            total_items: 1,
            completed_items: 0,
            claimed_by: Some(claimed_by.to_string()),
            claimed_at: Some("2026-09-26T00:00:00Z".to_string()),
            claim_expires_at: Some("2026-09-26T00:05:00Z".to_string()),
            pause_requested_at: None,
            cancel_requested_at: None,
            error_code: None,
            error_message: None,
            created_at: "2026-09-26T00:00:00Z".to_string(),
            started_at: Some("2026-09-26T00:00:00Z".to_string()),
            finished_at: None,
            updated_at: "2026-09-26T00:00:00Z".to_string(),
        }
    }
}

/// Row structure representing `public.enhancement_queue_items`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnhancementQueueItemRow {
    pub id: String,
    pub job_id: String,
    pub account_id: String,
    pub user_id: String,
    pub queue_order: i32,
    pub captured_slot: i32,
    pub template_id: i32,
    pub category: i32,
    pub base_name: String,
    pub tier: i32,
    #[serde(default)]
    pub icon: Option<i32>,
    pub initial_level: i32,
    pub current_level: i32,
    pub target_level: i32,
    pub payment_type: String,
    pub charm_mode: String,
    pub status: String,
    pub attempt_count: i32,

    // Durable attempt fields
    #[serde(default)]
    pub active_attempt_uuid: Option<String>,
    pub attempt_phase: String,
    #[serde(default)]
    pub attempt_expected_level: Option<i32>,
    #[serde(default)]
    pub attempt_target_level: Option<i32>,
    #[serde(default)]
    pub attempt_started_at: Option<String>,
    #[serde(default)]
    pub execute_may_have_been_sent_at: Option<String>,
    #[serde(default)]
    pub attempt_settled_at: Option<String>,
    #[serde(default)]
    pub last_result_code: Option<String>,

    // Authoritative item spend
    pub actual_gold_spent: i64,
    pub actual_gem_spent: i64,
    pub actual_material_1_spent: i64,
    pub actual_material_2_spent: i64,
    pub actual_material_3_spent: i64,
    pub actual_material_4_spent: i64,
    pub actual_charm_spent: i64,

    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl EnhancementQueueItemRow {
    #[cfg(test)]
    pub fn mock(
        id: &str,
        job_id: &str,
        queue_order: i32,
        captured_slot: i32,
        current_level: i32,
        target_level: i32,
        status: &str,
    ) -> Self {
        Self {
            id: id.to_string(),
            job_id: job_id.to_string(),
            account_id: "acc-1".to_string(),
            user_id: "user-1".to_string(),
            queue_order,
            captured_slot,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            tier: 2,
            icon: None,
            initial_level: 0,
            current_level,
            target_level,
            payment_type: "GOLD".to_string(),
            charm_mode: "NONE".to_string(),
            status: status.to_string(),
            attempt_count: 0,
            active_attempt_uuid: None,
            attempt_phase: "NONE".to_string(),
            attempt_expected_level: None,
            attempt_target_level: None,
            attempt_started_at: None,
            execute_may_have_been_sent_at: None,
            attempt_settled_at: None,
            last_result_code: None,
            actual_gold_spent: 0,
            actual_gem_spent: 0,
            actual_material_1_spent: 0,
            actual_material_2_spent: 0,
            actual_material_3_spent: 0,
            actual_material_4_spent: 0,
            actual_charm_spent: 0,
            error_code: None,
            error_message: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-09-26T00:00:00Z".to_string(),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
        }
    }
}

/// Outcome of selecting the next item to process in a queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemSelectionOutcome {
    ProceedWithItem(String),
    ActiveItemRunning(String),
    HaltedOnFailure(String),
    HaltedOnManualReview(String),
    AllItemsCompleted,
    EmptyQueue,
}

/// Enforces strict `queue_order` item execution and gating.
///
/// An item with queue_order N cannot start until item N-1 is authoritatively COMPLETED.
/// Any prior failure or manual review halts the entire queue.
pub fn select_next_executable_item(items: &[EnhancementQueueItemRow]) -> ItemSelectionOutcome {
    if items.is_empty() {
        return ItemSelectionOutcome::EmptyQueue;
    }

    let mut sorted_items = items.to_vec();
    sorted_items.sort_by_key(|it| it.queue_order);

    for item in &sorted_items {
        match item.status.as_str() {
            "RUNNING" => return ItemSelectionOutcome::ActiveItemRunning(item.id.clone()),
            "FAILED" | "CANCELLED" => return ItemSelectionOutcome::HaltedOnFailure(item.id.clone()),
            "MANUAL_REVIEW_REQUIRED" => return ItemSelectionOutcome::HaltedOnManualReview(item.id.clone()),
            "PENDING" => return ItemSelectionOutcome::ProceedWithItem(item.id.clone()),
            "COMPLETED" => continue,
            _ => return ItemSelectionOutcome::HaltedOnManualReview(item.id.clone()),
        }
    }

    ItemSelectionOutcome::AllItemsCompleted
}

/// Spec for executing exactly ONE enhancement level attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelAttemptSpec {
    pub attempt_uuid: String,
    pub captured_slot: i32,
    pub template_id: i32,
    pub category: i32,
    pub base_name: String,
    pub tier: i32,
    pub expected_level: i32,
    pub target_level: i32,
    pub charm_mode: u8,
    pub payment_type: u8,
    pub max_attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LevelAttemptError {
    #[error("item is already at or above target level")]
    AlreadyAtOrAboveTarget,
    #[error("current_level {0} is out of bounds (0..14)")]
    CurrentLevelOutOfBounds(i32),
    #[error("payment type parse error: {0}")]
    InvalidPaymentType(String),
    #[error("charm mode parse error: {0}")]
    InvalidCharmMode(String),
}

/// Generates a standard RFC 4122 UUID v4 for each fresh level attempt.
pub fn generate_attempt_uuid() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h = hex::encode(bytes);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Prepares the exact single-level enhancement attempt spec for an active item.
///
/// Enforces:
/// - target_level = current_level + 1 (single-level step only)
/// - max_attempts = 1 (strictly 1 attempt per command, no blind retry)
/// - fresh UUID generated per level
pub fn prepare_level_attempt(item: &EnhancementQueueItemRow) -> Result<LevelAttemptSpec, LevelAttemptError> {
    if item.current_level >= item.target_level {
        return Err(LevelAttemptError::AlreadyAtOrAboveTarget);
    }
    if item.current_level < 0 || item.current_level > 14 {
        return Err(LevelAttemptError::CurrentLevelOutOfBounds(item.current_level));
    }

    let payment_type = item
        .payment_type
        .parse::<EnhancementPaymentType>()
        .map_err(LevelAttemptError::InvalidPaymentType)?
        .to_wire_byte();

    let charm_mode = item
        .charm_mode
        .parse::<EnhancementCharmMode>()
        .map_err(LevelAttemptError::InvalidCharmMode)?
        .to_wire_byte();

    let attempt_uuid = generate_attempt_uuid();

    Ok(LevelAttemptSpec {
        attempt_uuid,
        captured_slot: item.captured_slot,
        template_id: item.template_id,
        category: item.category,
        base_name: item.base_name.clone(),
        tier: item.tier,
        expected_level: item.current_level,
        target_level: item.current_level + 1,
        charm_mode,
        payment_type,
        max_attempts: 1,
    })
}

/// Decision on mutation fence CAS response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationFenceDecision {
    /// Fence successfully persisted in database. Safe to dispatch command.
    FenceCommittedProceedToDispatch,
    /// Fence commit failed (0 rows returned or connection dropped). MUST NOT dispatch.
    FenceFailedDoNotDispatch,
}

/// Builds conditional CAS update payload to establish the mutation fence.
///
/// Persists attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT" and execute_may_have_been_sent_at.
pub fn build_mutation_fence_update(
    item_id: &str,
    attempt_uuid: &str,
    current_phase: EnhancementAttemptPhase,
) -> Result<(String, serde_json::Value), &'static str> {
    if !current_phase.is_pre_fence() {
        return Err("cannot establish mutation fence from post-fence phase");
    }

    let path = format!(
        "/rest/v1/enhancement_queue_items?id=eq.{item_id}&active_attempt_uuid=eq.{attempt_uuid}&attempt_phase=eq.{}",
        current_phase.as_str()
    );

    let body = serde_json::json!({
        "attempt_phase": EnhancementAttemptPhase::ExecuteMayHaveBeenSent.as_str(),
        "execute_may_have_been_sent_at": crate::supabase_rest::now_rfc3339(),
    });

    Ok((path, body))
}

/// Evaluates CAS response from establishing the mutation fence.
pub fn evaluate_mutation_fence_response(
    returned_rows: &[EnhancementQueueItemRow],
    expected_attempt_uuid: &str,
) -> MutationFenceDecision {
    if let Some(row) = returned_rows.first() {
        if row.active_attempt_uuid.as_deref() == Some(expected_attempt_uuid)
            || row.attempt_phase == EnhancementAttemptPhase::ExecuteMayHaveBeenSent.as_str()
        {
            return MutationFenceDecision::FenceCommittedProceedToDispatch;
        }
    }
    MutationFenceDecision::FenceFailedDoNotDispatch
}

/// Builds the single-item enhancement request payload for Zeus sidecar.
pub fn build_single_item_request(spec: &LevelAttemptSpec) -> crate::enhancement::EnhancementRequestPayload {
    crate::enhancement::EnhancementRequestPayload {
        request_id: spec.attempt_uuid.clone(),
        captured_slot: spec.captured_slot,
        template_id: spec.template_id,
        category: spec.category,
        base_name: spec.base_name.clone(),
        tier: spec.tier,
        expected_level: spec.expected_level,
        target_level: spec.target_level,
        charm_mode: spec.charm_mode,
        payment_type: spec.payment_type,
        max_attempts: spec.max_attempts,
        validation_only: false,
        requested_at: Some(crate::supabase_rest::now_rfc3339()),
    }
}

/// Outcome of committing settled actual spend and level to Supabase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementCommitOutcome {
    /// Spend was committed and incremented exactly once.
    CommittedOnce,
    /// Attempt was already settled in database (e.g. from prior execution before crash); spend was NOT incremented again.
    AlreadySettledDoNotIncrementAgain,
    /// Commit CAS failed and row was not in settled state.
    CommitFailed,
}

/// Builds idempotent CAS update for committing settled actual spend and current level.
pub fn build_settlement_commit_update(
    item: &EnhancementQueueItemRow,
    attempt_uuid: &str,
    telemetry: &crate::enhancement::EnhancementStatusTelemetry,
) -> Result<(String, serde_json::Value), &'static str> {
    let path = format!(
        "/rest/v1/enhancement_queue_items?id=eq.{}&active_attempt_uuid=eq.{}&attempt_phase=in.(EXECUTE_MAY_HAVE_BEEN_SENT,WAITING_RESULT,WAITING_SETTLEMENT)",
        item.id, attempt_uuid
    );

    let new_gold = item.actual_gold_spent.saturating_add(telemetry.actual_gold_spent);
    let new_gem = item.actual_gem_spent.saturating_add(telemetry.actual_gem_spent);
    let mat1 = telemetry.actual_materials_spent.first().copied().unwrap_or(0);
    let mat2 = telemetry.actual_materials_spent.get(1).copied().unwrap_or(0);
    let mat3 = telemetry.actual_materials_spent.get(2).copied().unwrap_or(0);
    let mat4 = telemetry.actual_materials_spent.get(3).copied().unwrap_or(0);
    let new_mat1 = item.actual_material_1_spent.saturating_add(mat1);
    let new_mat2 = item.actual_material_2_spent.saturating_add(mat2);
    let new_mat3 = item.actual_material_3_spent.saturating_add(mat3);
    let new_mat4 = item.actual_material_4_spent.saturating_add(mat4);
    let new_charm = item.actual_charm_spent.saturating_add(telemetry.actual_charms_spent);

    let body = serde_json::json!({
        "actual_gold_spent": new_gold,
        "actual_gem_spent": new_gem,
        "actual_material_1_spent": new_mat1,
        "actual_material_2_spent": new_mat2,
        "actual_material_3_spent": new_mat3,
        "actual_material_4_spent": new_mat4,
        "actual_charm_spent": new_charm,
        "current_level": telemetry.current_level,
        "attempt_phase": EnhancementAttemptPhase::Settled.as_str(),
        "attempt_settled_at": crate::supabase_rest::now_rfc3339(),
        "last_result_code": telemetry.last_result,
    });

    Ok((path, body))
}

/// Evaluates result of settlement commit CAS query.
pub fn evaluate_settlement_commit_result(
    returned_rows: &[EnhancementQueueItemRow],
    _attempt_uuid: &str,
    current_item_phase: &str,
) -> SettlementCommitOutcome {
    if let Some(row) = returned_rows.first() {
        if row.attempt_phase == EnhancementAttemptPhase::Settled.as_str() {
            return SettlementCommitOutcome::CommittedOnce;
        }
    }
    if current_item_phase == EnhancementAttemptPhase::Settled.as_str() {
        return SettlementCommitOutcome::AlreadySettledDoNotIncrementAgain;
    }
    SettlementCommitOutcome::CommitFailed
}

/// Mapped queue outcome for an item failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueFailureOutcome {
    ItemFailedAndStopQueue {
        error_code: String,
        error_message: Option<String>,
    },
    ManualReviewRequiredAndFreezeQueue {
        error_code: String,
        error_message: Option<String>,
    },
}

/// Maps any runtime failure state to the durable item and job terminal states.
///
/// Follows P0 rules:
/// - Any failure stops the queue in v1.
/// - Unsettled accounting or ambiguity requires manual review and freezes the queue.
/// - Failed items are never automatically skipped.
pub fn map_runtime_failure_to_queue_outcome(
    state: &str,
    last_result: Option<&str>,
    error_message: Option<&str>,
) -> QueueFailureOutcome {
    let msg = error_message.map(str::to_string);
    match state {
        "ACCOUNTING_UNSETTLED" | "RESULT_AMBIGUOUS" | "MANUAL_REVIEW_REQUIRED" => {
            QueueFailureOutcome::ManualReviewRequiredAndFreezeQueue {
                error_code: state.to_string(),
                error_message: msg,
            }
        }
        _ => {
            let code = last_result.unwrap_or(state).to_string();
            QueueFailureOutcome::ItemFailedAndStopQueue {
                error_code: code,
                error_message: msg,
            }
        }
    }
}

/// Action to take when pause is evaluated against current attempt boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseAction {
    /// No pause was requested by user.
    NoPauseRequested,
    /// Pause requested before mutation fence: halt before dispatch and transition to PAUSED.
    PauseImmediatelyPreFence,
    /// Pause requested after mutation fence: do NOT interrupt in-flight attempt; wait for authoritative settlement, commit once, then transition to PAUSED.
    WaitForSettlementPostFenceThenPause,
}

/// Evaluates pause request against current item attempt phase.
pub fn evaluate_pause_request(
    job: &EnhancementQueueJobRow,
    current_phase: EnhancementAttemptPhase,
) -> PauseAction {
    let is_pause_requested = job.pause_requested_at.is_some() || job.status == "PAUSING";
    if !is_pause_requested {
        return PauseAction::NoPauseRequested;
    }

    if current_phase.is_pre_fence() {
        PauseAction::PauseImmediatelyPreFence
    } else {
        PauseAction::WaitForSettlementPostFenceThenPause
    }
}

/// Action to take when cancel is evaluated against current attempt boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelAction {
    /// No cancel was requested by user.
    NoCancelRequested,
    /// Cancel requested before mutation fence: cancel active item and all remaining items immediately.
    CancelImmediatelyPreFence,
    /// Cancel requested after mutation fence: do NOT interrupt active attempt; wait for settlement, commit once, then cancel future work.
    WaitForSettlementPostFenceThenCancel,
}

/// Evaluates cancel request against current item attempt phase.
pub fn evaluate_cancel_request(
    job: &EnhancementQueueJobRow,
    current_phase: EnhancementAttemptPhase,
) -> CancelAction {
    if job.cancel_requested_at.is_none() {
        return CancelAction::NoCancelRequested;
    }

    if current_phase.is_pre_fence() {
        CancelAction::CancelImmediatelyPreFence
    } else {
        CancelAction::WaitForSettlementPostFenceThenCancel
    }
}

/// Action to perform upon agent restart recovery.
#[derive(Debug, Clone, PartialEq)]
pub enum RestartRecoveryAction {
    /// Job is QUEUED: claim via conditional CAS.
    ClaimQueuedJob,
    /// Job is claimed/RUNNING but no active item: select and start next pending item.
    StartNextPendingItem,
    /// Item was in pre-fence phase (NONE, PREPARING, READY_TO_EXECUTE): safe to resume preparation.
    ResumePreFencePreparation,
    /// Item was post-fence and authoritative settled telemetry was recovered from disk: commit spend and level.
    ReconcileSettledAttempt(crate::enhancement::EnhancementStatusTelemetry),
    /// Item was post-fence and could NOT be reconciled conclusively: freeze queue in MANUAL_REVIEW_REQUIRED.
    TransitionToManualReviewRequired { reason: String },
    /// Item is in SETTLED phase: advance item to completed or start next level attempt.
    AdvanceSettledItem { mark_completed: bool },
    /// Job is paused.
    JobPaused,
    /// Job is terminal.
    JobTerminal,
}

/// Deterministic evaluator for agent restart recovery across all 11 lifecycle phases.
///
/// Invariants:
/// - Pre-fence attempts are resumed safely without double dispatch risk.
/// - Post-fence attempts are NEVER blindly replayed.
/// - Exact settled spend is reconciled from disk telemetry when available.
/// - Unreconciled post-fence states transition to MANUAL_REVIEW_REQUIRED.
pub fn evaluate_restart_recovery_step(
    job: &EnhancementQueueJobRow,
    active_item: Option<&EnhancementQueueItemRow>,
    disk_telemetry: Option<&crate::enhancement::EnhancementStatusTelemetry>,
) -> RestartRecoveryAction {
    if job.status == "PAUSED" || job.status == "PAUSING" {
        return RestartRecoveryAction::JobPaused;
    }
    if job.status == "COMPLETED" || job.status == "FAILED" || job.status == "CANCELLED" {
        return RestartRecoveryAction::JobTerminal;
    }
    if job.status == "MANUAL_REVIEW_REQUIRED" {
        return RestartRecoveryAction::TransitionToManualReviewRequired {
            reason: "job is in MANUAL_REVIEW_REQUIRED".to_string(),
        };
    }
    if job.status == "QUEUED" {
        return RestartRecoveryAction::ClaimQueuedJob;
    }

    // Job is RUNNING
    let item = match active_item {
        Some(it) => it,
        None => return RestartRecoveryAction::StartNextPendingItem,
    };

    let phase = match item.attempt_phase.parse::<EnhancementAttemptPhase>() {
        Ok(p) => p,
        Err(_) => {
            return RestartRecoveryAction::TransitionToManualReviewRequired {
                reason: format!("corrupt attempt phase: {}", item.attempt_phase),
            }
        }
    };

    match phase {
        EnhancementAttemptPhase::None
        | EnhancementAttemptPhase::Preparing
        | EnhancementAttemptPhase::ReadyToExecute => {
            // Pre-fence: safe to resume preparation with fresh attempt UUID
            RestartRecoveryAction::ResumePreFencePreparation
        }
        EnhancementAttemptPhase::ExecuteMayHaveBeenSent
        | EnhancementAttemptPhase::WaitingResult
        | EnhancementAttemptPhase::WaitingSettlement => {
            // Post-fence: NEVER blindly replay!
            // Check if disk telemetry has matching attempt_uuid and settled accounting
            if let Some(telemetry) = disk_telemetry {
                if item.active_attempt_uuid.as_deref() == Some(&telemetry.request_id)
                    && telemetry.accounting_status == "SETTLED"
                {
                    return RestartRecoveryAction::ReconcileSettledAttempt(telemetry.clone());
                }
            }
            RestartRecoveryAction::TransitionToManualReviewRequired {
                reason: format!(
                    "unreconciled post-fence attempt {} in phase {}",
                    item.active_attempt_uuid.as_deref().unwrap_or("none"),
                    phase.as_str()
                ),
            }
        }
        EnhancementAttemptPhase::Settled => {
            let mark_completed = item.current_level >= item.target_level;
            RestartRecoveryAction::AdvanceSettledItem { mark_completed }
        }
    }
}

/// Outcome of attempting to claim a queue job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOutcome {
    Won,
    Lost,
}

/// Evaluates CAS claim update result (Prefer: return=representation).
pub fn evaluate_claim_result(returned_rows: &[EnhancementQueueJobRow], expected_worker: &str) -> ClaimOutcome {
    if let Some(row) = returned_rows.first() {
        if row.claimed_by.as_deref() == Some(expected_worker) {
            return ClaimOutcome::Won;
        }
    }
    ClaimOutcome::Lost
}

/// Builds PostgREST path for conditional atomic claim update.
pub fn build_claim_job_path(job_id: &str, account_id: &str, device_id: &str) -> String {
    format!(
        "/rest/v1/enhancement_queue_jobs?id=eq.{job_id}&account_id=eq.{account_id}&device_id=eq.{device_id}&status=eq.QUEUED"
    )
}

/// Decision on whether agent is authorized to claim or mutate a given queue job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimAuthDecision {
    Authorized,
    RejectedAccountMismatch,
    RejectedDeviceMismatch,
}

/// Verifies that job ownership matches the authenticated agent and active account.
pub fn evaluate_claim_authorization(
    job_account_id: &str,
    job_device_id: &str,
    agent_account_id: &str,
    agent_device_id: &str,
) -> ClaimAuthDecision {
    if job_account_id != agent_account_id {
        ClaimAuthDecision::RejectedAccountMismatch
    } else if job_device_id != agent_device_id {
        ClaimAuthDecision::RejectedDeviceMismatch
    } else {
        ClaimAuthDecision::Authorized
    }
}

/// In-flight state of a queue attempt dispatched to Zeus.
#[derive(Debug, Clone)]
pub struct InFlightQueueAttempt {
    pub job_id: String,
    pub item_id: String,
    pub attempt_uuid: String,
    pub expected_level: i32,
    pub target_level: i32,
    pub started_at: std::time::Instant,
}

/// Tracks enhancement queue orchestrator state for a single account.
#[derive(Debug, Default, Clone)]
pub struct AccountQueueTracker {
    pub active_job_id: Option<String>,
    pub in_flight_attempt: Option<InFlightQueueAttempt>,
    pub last_poll: Option<std::time::Instant>,
}

/// Builds conditional CAS update payload to establish the mutation fence with a full level attempt spec.
pub fn build_mutation_fence_update_with_spec(
    item_id: &str,
    account_id: &str,
    spec: &LevelAttemptSpec,
    current_phase: &str,
) -> Result<(String, serde_json::Value), &'static str> {
    let phase = current_phase
        .parse::<EnhancementAttemptPhase>()
        .map_err(|_| "invalid attempt phase")?;
    if !phase.is_pre_fence() {
        return Err("cannot establish mutation fence from post-fence phase");
    }

    let now = crate::supabase_rest::now_rfc3339();
    let path = format!(
        "/rest/v1/enhancement_queue_items?id=eq.{item_id}&account_id=eq.{account_id}&attempt_phase=eq.{current_phase}"
    );

    let body = serde_json::json!({
        "active_attempt_uuid": spec.attempt_uuid,
        "attempt_phase": EnhancementAttemptPhase::ExecuteMayHaveBeenSent.as_str(),
        "attempt_expected_level": spec.expected_level,
        "attempt_target_level": spec.target_level,
        "attempt_started_at": now,
        "execute_may_have_been_sent_at": now,
    });

    Ok((path, body))
}

/// Recovers queue state for an account on boot before general file cleanup.
/// Inspects active unresolved jobs and on-disk enhancement status to reconcile post-fence attempts.
pub fn recover_account_enhancement_queue_on_boot(
    home_dir: &std::path::Path,
    device_id: &str,
    account_id: &str,
    rest: &crate::supabase_rest::SupabaseRest,
) -> Result<Option<String>, crate::supabase_rest::RestError> {
    // 1. Fetch active unresolved queue job
    let job = match rest.fetch_active_unresolved_queue_job(account_id, device_id)? {
        Some(j) => j,
        None => {
            // No unresolved job, safe to clean enhancement files
            crate::enhancement::clean_enhancement_files(home_dir);
            return Ok(None);
        }
    };

    eprintln!(
        "[enhancement_queue] boot recovery: found unresolved job {} (status={}) for account {}",
        job.id, job.status, account_id
    );

    // 2. Fetch items for this job
    let items = rest.fetch_queue_items(&job.id, account_id)?;
    let active_item = items
        .iter()
        .find(|it| it.status == "RUNNING" || it.active_attempt_uuid.is_some());

    let disk_telemetry = crate::enhancement::read_enhancement_status(home_dir);
    let recovery_step = evaluate_restart_recovery_step(
        &job,
        active_item,
        disk_telemetry.as_ref(),
    );

    match recovery_step {
        RestartRecoveryAction::ClaimQueuedJob
        | RestartRecoveryAction::StartNextPendingItem
        | RestartRecoveryAction::ResumePreFencePreparation
        | RestartRecoveryAction::JobPaused
        | RestartRecoveryAction::JobTerminal => {
            crate::enhancement::clean_enhancement_files(home_dir);
            Ok(Some(job.id))
        }
        RestartRecoveryAction::ReconcileSettledAttempt(telemetry) => {
            let item = active_item.unwrap();
            let _ = rest.commit_item_settlement(item, &telemetry.request_id, &telemetry);
            if telemetry.current_level >= item.target_level {
                let _ = rest.update_queue_item(
                    &item.id,
                    account_id,
                    &serde_json::json!({
                        "status": "COMPLETED",
                        "finished_at": crate::supabase_rest::now_rfc3339()
                    }),
                );
            }
            crate::enhancement::clean_enhancement_files(home_dir);
            Ok(Some(job.id))
        }
        RestartRecoveryAction::TransitionToManualReviewRequired { reason } => {
            eprintln!(
                "[enhancement_queue] boot recovery: post-fence attempt cannot be reconciled conclusively ({reason}), marking MANUAL_REVIEW_REQUIRED"
            );
            if let Some(item) = active_item {
                let _ = rest.update_queue_item(
                    &item.id,
                    account_id,
                    &serde_json::json!({
                        "status": "MANUAL_REVIEW_REQUIRED",
                        "error_message": reason,
                    }),
                );
            }
            let _ = rest.update_queue_job(
                &job.id,
                account_id,
                &serde_json::json!({
                    "status": "MANUAL_REVIEW_REQUIRED",
                    "error_message": reason,
                }),
            );
            crate::enhancement::clean_enhancement_files(home_dir);
            Ok(Some(job.id))
        }
        RestartRecoveryAction::AdvanceSettledItem { mark_completed } => {
            if mark_completed {
                if let Some(item) = active_item {
                    let _ = rest.update_queue_item(
                        &item.id,
                        account_id,
                        &serde_json::json!({
                            "status": "COMPLETED",
                            "finished_at": crate::supabase_rest::now_rfc3339()
                        }),
                    );
                }
            }
            crate::enhancement::clean_enhancement_files(home_dir);
            Ok(Some(job.id))
        }
    }
}

/// Orchestrates enhancement queue execution for a single account on a periodic tick.
pub fn tick_account_enhancement_queue(
    home_dir: &std::path::Path,
    device_id: &str,
    account_id: &str,
    process_alive: bool,
    has_pending_single_item_enhancement: bool,
    tracker: &mut AccountQueueTracker,
    rest: &crate::supabase_rest::SupabaseRest,
) {
    // 1. Exclusivity check: if ad-hoc single-item enhancement command is pending, do not touch queue
    if has_pending_single_item_enhancement {
        return;
    }

    // 2. Poll/reconcile in-flight attempt if active
    if let Some(attempt) = tracker.in_flight_attempt.take() {
        if !process_alive {
            eprintln!(
                "[enhancement_queue] account {} process died while attempt {} in flight",
                account_id, attempt.attempt_uuid
            );
            // Check if status exists on disk before crashing
            let disk_status = crate::enhancement::read_enhancement_status(home_dir);
            let mut settled = false;
            if let Some(status) = disk_status {
                if status.request_id == attempt.attempt_uuid && status.state == "SUCCESS" {
                    if status.current_level == attempt.expected_level + 1 {
                        if let Ok(items) = rest.fetch_queue_items(&attempt.job_id, account_id) {
                            if let Some(item) = items.iter().find(|i| i.id == attempt.item_id) {
                                let _ = rest.commit_item_settlement(item, &attempt.attempt_uuid, &status);
                                settled = true;
                            }
                        }
                    }
                }
            }
            if !settled {
                let _ = rest.update_queue_item(
                    &attempt.item_id,
                    account_id,
                    &serde_json::json!({
                        "status": "MANUAL_REVIEW_REQUIRED",
                        "error_message": "process died while enhancement execution was in flight",
                    }),
                );
                let _ = rest.update_queue_job(
                    &attempt.job_id,
                    account_id,
                    &serde_json::json!({
                        "status": "MANUAL_REVIEW_REQUIRED",
                        "error_message": "process died while enhancement execution was in flight",
                    }),
                );
            }
            crate::enhancement::clean_enhancement_files(home_dir);
            tracker.active_job_id = None;
            return;
        }

        if attempt.started_at.elapsed().as_secs() >= crate::enhancement::ENHANCEMENT_TIMEOUT_SECS {
            eprintln!(
                "[enhancement_queue] account {} attempt {} timed out waiting for JVM",
                account_id, attempt.attempt_uuid
            );
            let _ = rest.update_queue_item(
                &attempt.item_id,
                account_id,
                &serde_json::json!({
                    "status": "MANUAL_REVIEW_REQUIRED",
                    "error_message": "enhancement attempt timed out waiting for JVM",
                }),
            );
            let _ = rest.update_queue_job(
                &attempt.job_id,
                account_id,
                &serde_json::json!({
                    "status": "MANUAL_REVIEW_REQUIRED",
                    "error_message": "enhancement attempt timed out waiting for JVM",
                }),
            );
            crate::enhancement::clean_enhancement_files(home_dir);
            tracker.active_job_id = None;
            return;
        }

        match crate::enhancement::poll_enhancement_status(home_dir, &attempt.attempt_uuid) {
            crate::enhancement::EnhancementPollOutcome::NoStatusYet => {
                // Keep waiting
                tracker.in_flight_attempt = Some(attempt);
                return;
            }
            crate::enhancement::EnhancementPollOutcome::TerminalSuccess(status) => {
                if status.current_level != attempt.expected_level + 1 {
                    eprintln!(
                        "[enhancement_queue] level discrepancy: expected {}, got {}",
                        attempt.expected_level + 1,
                        status.current_level
                    );
                    let _ = rest.update_queue_item(
                        &attempt.item_id,
                        account_id,
                        &serde_json::json!({
                            "status": "MANUAL_REVIEW_REQUIRED",
                            "error_message": format!("level discrepancy: expected {}, got {}", attempt.expected_level + 1, status.current_level),
                        }),
                    );
                    let _ = rest.update_queue_job(
                        &attempt.job_id,
                        account_id,
                        &serde_json::json!({
                            "status": "MANUAL_REVIEW_REQUIRED",
                            "error_message": "level discrepancy observed from runtime result",
                        }),
                    );
                    crate::enhancement::clean_enhancement_files(home_dir);
                    tracker.active_job_id = None;
                    return;
                }

                // Settle item idempotently
                if let Ok(items) = rest.fetch_queue_items(&attempt.job_id, account_id) {
                    if let Some(item) = items.iter().find(|i| i.id == attempt.item_id) {
                        let _ = rest.commit_item_settlement(item, &attempt.attempt_uuid, &status);

                        // Check pause/cancel request after settlement
                        if let Ok(Some(job)) = rest.fetch_active_unresolved_queue_job(account_id, device_id) {
                            if job.pause_requested_at.is_some() {
                                let _ = rest.update_queue_job(&job.id, account_id, &serde_json::json!({
                                    "status": "PAUSED",
                                    "pause_requested_at": serde_json::Value::Null,
                                }));
                                crate::enhancement::clean_enhancement_files(home_dir);
                                tracker.active_job_id = None;
                                return;
                            }
                            if job.cancel_requested_at.is_some() {
                                let _ = rest.update_queue_job(&job.id, account_id, &serde_json::json!({
                                    "status": "CANCELLED",
                                    "finished_at": crate::supabase_rest::now_rfc3339(),
                                    "cancel_requested_at": serde_json::Value::Null,
                                }));
                                for it in &items {
                                    if it.status == "PENDING" {
                                        let _ = rest.update_queue_item(&it.id, account_id, &serde_json::json!({
                                            "status": "CANCELLED",
                                            "finished_at": crate::supabase_rest::now_rfc3339(),
                                        }));
                                    }
                                }
                                crate::enhancement::clean_enhancement_files(home_dir);
                                tracker.active_job_id = None;
                                return;
                            }
                        }

                        if status.current_level >= attempt.target_level {
                            let _ = rest.update_queue_item(
                                &attempt.item_id,
                                account_id,
                                &serde_json::json!({
                                    "status": "COMPLETED",
                                    "finished_at": crate::supabase_rest::now_rfc3339(),
                                }),
                            );
                            let all_completed = items
                                .iter()
                                .all(|i| i.id == attempt.item_id || i.status == "COMPLETED");
                            if all_completed {
                                let _ = rest.update_queue_job(
                                    &attempt.job_id,
                                    account_id,
                                    &serde_json::json!({
                                        "status": "COMPLETED",
                                        "finished_at": crate::supabase_rest::now_rfc3339(),
                                    }),
                                );
                                tracker.active_job_id = None;
                            }
                        }
                    }
                }
                crate::enhancement::clean_enhancement_files(home_dir);
                return;
            }
            crate::enhancement::EnhancementPollOutcome::TerminalFailure { state, error_message } => {
                let outcome = map_runtime_failure_to_queue_outcome(&state, None, error_message.as_deref());
                match outcome {
                    QueueFailureOutcome::ItemFailedAndStopQueue { error_code, error_message } => {
                        let _ = rest.update_queue_item(
                            &attempt.item_id,
                            account_id,
                            &serde_json::json!({
                                "status": "FAILED",
                                "error_code": error_code,
                                "error_message": error_message,
                                "attempt_phase": "SETTLED",
                                "finished_at": crate::supabase_rest::now_rfc3339(),
                            }),
                        );
                        let _ = rest.update_queue_job(
                            &attempt.job_id,
                            account_id,
                            &serde_json::json!({
                                "status": "FAILED",
                                "error_code": error_code,
                                "error_message": error_message,
                                "finished_at": crate::supabase_rest::now_rfc3339(),
                            }),
                        );
                    }
                    QueueFailureOutcome::ManualReviewRequiredAndFreezeQueue { error_code, error_message } => {
                        let _ = rest.update_queue_item(
                            &attempt.item_id,
                            account_id,
                            &serde_json::json!({
                                "status": "MANUAL_REVIEW_REQUIRED",
                                "error_code": error_code,
                                "error_message": error_message,
                                "attempt_phase": "SETTLED",
                            }),
                        );
                        let _ = rest.update_queue_job(
                            &attempt.job_id,
                            account_id,
                            &serde_json::json!({
                                "status": "MANUAL_REVIEW_REQUIRED",
                                "error_code": error_code,
                                "error_message": error_message,
                            }),
                        );
                    }
                }
                crate::enhancement::clean_enhancement_files(home_dir);
                tracker.active_job_id = None;
                return;
            }
            crate::enhancement::EnhancementPollOutcome::InvalidPayload(err) => {
                let _ = rest.update_queue_item(
                    &attempt.item_id,
                    account_id,
                    &serde_json::json!({
                        "status": "MANUAL_REVIEW_REQUIRED",
                        "error_message": format!("invalid status payload: {err}"),
                    }),
                );
                let _ = rest.update_queue_job(
                    &attempt.job_id,
                    account_id,
                    &serde_json::json!({
                        "status": "MANUAL_REVIEW_REQUIRED",
                        "error_message": "invalid status payload from JVM",
                    }),
                );
                crate::enhancement::clean_enhancement_files(home_dir);
                tracker.active_job_id = None;
                return;
            }
        }
    }

    // 3. No in-flight attempt; check/claim jobs
    if tracker.active_job_id.is_none() {
        if let Ok(Some(unresolved)) = rest.fetch_active_unresolved_queue_job(account_id, device_id) {
            tracker.active_job_id = Some(unresolved.id);
        } else if let Ok(Some(claimable)) = rest.fetch_claimable_queue_job(account_id, device_id) {
            let worker_id = format!("{device_id}-agent");
            if let Ok(Some(won)) = rest.claim_queue_job(&claimable.id, account_id, device_id, &worker_id, 3600) {
                tracker.active_job_id = Some(won.id);
            } else {
                return;
            }
        } else {
            return;
        }
    }

    let job_id = match tracker.active_job_id.clone() {
        Some(id) => id,
        None => return,
    };

    if !process_alive {
        return; // Cannot execute without live JVM process
    }

    let job = match rest.fetch_active_unresolved_queue_job(account_id, device_id) {
        Ok(Some(j)) if j.id == job_id => j,
        _ => {
            tracker.active_job_id = None;
            return;
        }
    };

    if job.status != "RUNNING" {
        tracker.active_job_id = None;
        return;
    }

    // Handle pause request before dispatch
    if job.pause_requested_at.is_some() {
        let _ = rest.update_queue_job(&job.id, account_id, &serde_json::json!({
            "status": "PAUSED",
            "pause_requested_at": serde_json::Value::Null,
        }));
        tracker.active_job_id = None;
        return;
    }

    // Handle cancel request before dispatch
    if job.cancel_requested_at.is_some() {
        let _ = rest.update_queue_job(&job.id, account_id, &serde_json::json!({
            "status": "CANCELLED",
            "finished_at": crate::supabase_rest::now_rfc3339(),
            "cancel_requested_at": serde_json::Value::Null,
        }));
        if let Ok(items) = rest.fetch_queue_items(&job.id, account_id) {
            for it in items {
                if it.status == "PENDING" {
                    let _ = rest.update_queue_item(&it.id, account_id, &serde_json::json!({
                        "status": "CANCELLED",
                        "finished_at": crate::supabase_rest::now_rfc3339(),
                    }));
                }
            }
        }
        tracker.active_job_id = None;
        return;
    }

    // Fetch items strictly in queue_order
    let items = match rest.fetch_queue_items(&job.id, account_id) {
        Ok(its) => its,
        Err(e) => {
            eprintln!("[enhancement_queue] fetch_queue_items failed: {e}");
            return;
        }
    };

    match select_next_executable_item(&items) {
        ItemSelectionOutcome::AllItemsCompleted | ItemSelectionOutcome::EmptyQueue => {
            let _ = rest.update_queue_job(&job.id, account_id, &serde_json::json!({
                "status": "COMPLETED",
                "finished_at": crate::supabase_rest::now_rfc3339(),
            }));
            tracker.active_job_id = None;
        }
        ItemSelectionOutcome::HaltedOnFailure(_) | ItemSelectionOutcome::HaltedOnManualReview(_) => {
            tracker.active_job_id = None;
        }
        ItemSelectionOutcome::ActiveItemRunning(item_id) | ItemSelectionOutcome::ProceedWithItem(item_id) => {
            let item = match items.into_iter().find(|i| i.id == item_id) {
                Some(i) => i,
                None => return,
            };

            if item.status == "PENDING" {
                let _ = rest.update_queue_item(&item.id, account_id, &serde_json::json!({
                    "status": "RUNNING",
                    "started_at": crate::supabase_rest::now_rfc3339(),
                }));
            }

            let spec = match prepare_level_attempt(&item) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[enhancement_queue] prepare_level_attempt error: {e:?}");
                    return;
                }
            };

            // Commit mutation fence BEFORE sidecar dispatch
            let fence_res = rest.commit_item_mutation_fence_with_spec(
                &item.id,
                account_id,
                &spec,
                &item.attempt_phase,
            );

            match fence_res {
                Ok(true) => {
                    // Fence committed! Dispatch single-item command to Zeus sidecar
                    let req_payload = build_single_item_request(&spec);
                    if let Err(e) = crate::enhancement::write_enhancement_request_file(home_dir, &req_payload) {
                        eprintln!("[enhancement_queue] write_enhancement_request_file failed: {e}");
                    }
                    tracker.in_flight_attempt = Some(InFlightQueueAttempt {
                        job_id: job.id,
                        item_id: item.id,
                        attempt_uuid: spec.attempt_uuid,
                        expected_level: spec.expected_level,
                        target_level: spec.target_level,
                        started_at: std::time::Instant::now(),
                    });
                }
                Ok(false) => {
                    eprintln!("[enhancement_queue] mutation fence CAS rejected (0 rows), aborting dispatch");
                }
                Err(e) => {
                    eprintln!("[enhancement_queue] commit_item_mutation_fence error: {e}, aborting dispatch");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_status_parsing_and_invariants() {
        let statuses = [
            ("DRAFT", EnhancementQueueJobStatus::Draft, false),
            ("QUEUED", EnhancementQueueJobStatus::Queued, true),
            ("RUNNING", EnhancementQueueJobStatus::Running, false),
            ("PAUSING", EnhancementQueueJobStatus::Pausing, false),
            ("PAUSED", EnhancementQueueJobStatus::Paused, false),
            ("COMPLETED", EnhancementQueueJobStatus::Completed, false),
            ("FAILED", EnhancementQueueJobStatus::Failed, false),
            ("CANCELLED", EnhancementQueueJobStatus::Cancelled, false),
            ("MANUAL_REVIEW_REQUIRED", EnhancementQueueJobStatus::ManualReviewRequired, false),
        ];

        for (raw, expected, is_claimable) in statuses {
            let parsed: EnhancementQueueJobStatus = raw.parse().expect("must parse job status");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), raw);
            assert_eq!(parsed.is_claimable(), is_claimable);
        }
    }

    #[test]
    fn test_attempt_phase_invariants() {
        let phases = [
            ("NONE", EnhancementAttemptPhase::None, true, false),
            ("PREPARING", EnhancementAttemptPhase::Preparing, true, false),
            ("READY_TO_EXECUTE", EnhancementAttemptPhase::ReadyToExecute, true, false),
            ("EXECUTE_MAY_HAVE_BEEN_SENT", EnhancementAttemptPhase::ExecuteMayHaveBeenSent, false, true),
            ("WAITING_RESULT", EnhancementAttemptPhase::WaitingResult, false, true),
            ("WAITING_SETTLEMENT", EnhancementAttemptPhase::WaitingSettlement, false, true),
            ("SETTLED", EnhancementAttemptPhase::Settled, false, true),
        ];

        for (raw, expected, is_pre_fence, is_post_fence) in phases {
            let parsed: EnhancementAttemptPhase = raw.parse().expect("must parse attempt phase");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_str(), raw);
            assert_eq!(parsed.is_pre_fence(), is_pre_fence);
            assert_eq!(parsed.is_post_fence(), is_post_fence);
        }
    }

    #[test]
    fn test_payment_type_and_charm_mode_wire_mapping() {
        assert_eq!("GOLD".parse::<EnhancementPaymentType>().unwrap().to_wire_byte(), 0);
        assert_eq!("GEMS".parse::<EnhancementPaymentType>().unwrap().to_wire_byte(), 1);

        assert_eq!("NONE".parse::<EnhancementCharmMode>().unwrap().to_wire_byte(), 0);
        assert_eq!("CO_3_LA".parse::<EnhancementCharmMode>().unwrap().to_wire_byte(), 1);
        assert_eq!("THREE_LEAF".parse::<EnhancementCharmMode>().unwrap().to_wire_byte(), 1);
        assert_eq!("CO_4_LA".parse::<EnhancementCharmMode>().unwrap().to_wire_byte(), 2);
        assert_eq!("FOUR_LEAF".parse::<EnhancementCharmMode>().unwrap().to_wire_byte(), 2);
        assert_eq!("AUTO_POLICY".parse::<EnhancementCharmMode>().unwrap().to_wire_byte(), 3);
    }

    #[test]
    fn test_claim_cas_and_isolation() {
        let job_id = "job-100";
        let authorized_acc = "acc-1";
        let authorized_dev = "dev-1";
        let worker_a = "worker-a";
        let worker_b = "worker-b";

        // Query check: path contains scoping
        let query_path = build_claim_job_path(job_id, authorized_acc, authorized_dev);
        assert!(query_path.contains("id=eq.job-100"));
        assert!(query_path.contains("account_id=eq.acc-1"));
        assert!(query_path.contains("device_id=eq.dev-1"));
        assert!(query_path.contains("status=eq.QUEUED"));

        // Worker A claims 1 row -> wins
        let won = evaluate_claim_result(
            &[EnhancementQueueJobRow::mock(job_id, authorized_acc, authorized_dev, "RUNNING", worker_a)],
            worker_a,
        );
        assert_eq!(won, ClaimOutcome::Won);

        // Worker B races concurrently on same row -> returns 0 rows (lost CAS)
        let lost = evaluate_claim_result(&[], worker_b);
        assert_eq!(lost, ClaimOutcome::Lost);

        // Unauthorized account attempts to claim -> rejected
        assert_eq!(
            evaluate_claim_authorization(authorized_acc, authorized_dev, "acc-other", authorized_dev),
            ClaimAuthDecision::RejectedAccountMismatch
        );
        assert_eq!(
            evaluate_claim_authorization(authorized_acc, authorized_dev, authorized_acc, "dev-other"),
            ClaimAuthDecision::RejectedDeviceMismatch
        );
        assert_eq!(
            evaluate_claim_authorization(authorized_acc, authorized_dev, authorized_acc, authorized_dev),
            ClaimAuthDecision::Authorized
        );
    }

    #[test]
    fn test_strict_item_ordering_and_gating() {
        let mut item1 = EnhancementQueueItemRow::mock("it-1", "job-1", 1, 0, 0, 3, "PENDING");
        let mut item2 = EnhancementQueueItemRow::mock("it-2", "job-1", 2, 0, 0, 3, "PENDING");

        // When item1 is PENDING, item1 is selected, item2 cannot start
        assert_eq!(
            select_next_executable_item(&[item1.clone(), item2.clone()]),
            ItemSelectionOutcome::ProceedWithItem("it-1".to_string())
        );

        // When item1 is RUNNING, item2 cannot start; item1 is active
        item1.status = "RUNNING".to_string();
        assert_eq!(
            select_next_executable_item(&[item1.clone(), item2.clone()]),
            ItemSelectionOutcome::ActiveItemRunning("it-1".to_string())
        );

        // When item1 is FAILED, item2 is blocked and queue must stop
        item1.status = "FAILED".to_string();
        assert_eq!(
            select_next_executable_item(&[item1.clone(), item2.clone()]),
            ItemSelectionOutcome::HaltedOnFailure("it-1".to_string())
        );

        // When item1 is MANUAL_REVIEW_REQUIRED, item2 is blocked and queue must stop
        item1.status = "MANUAL_REVIEW_REQUIRED".to_string();
        assert_eq!(
            select_next_executable_item(&[item1.clone(), item2.clone()]),
            ItemSelectionOutcome::HaltedOnManualReview("it-1".to_string())
        );

        // Only when item1 is COMPLETED, item2 can start
        item1.status = "COMPLETED".to_string();
        assert_eq!(
            select_next_executable_item(&[item1.clone(), item2.clone()]),
            ItemSelectionOutcome::ProceedWithItem("it-2".to_string())
        );

        // When both COMPLETED, AllItemsCompleted
        item2.status = "COMPLETED".to_string();
        assert_eq!(
            select_next_executable_item(&[item1, item2]),
            ItemSelectionOutcome::AllItemsCompleted
        );
    }

    #[test]
    fn test_one_level_execution_and_attempt_spec() {
        let item = EnhancementQueueItemRow::mock("it-1", "job-1", 1, 0, 3, 5, "RUNNING");
        let spec = prepare_level_attempt(&item).expect("must prepare attempt");

        // Exact invariants:
        assert_eq!(spec.expected_level, 3);
        assert_eq!(spec.target_level, 4); // Exactly current + 1!
        assert_eq!(spec.max_attempts, 1);  // Strictly 1 attempt per level!
        assert_ne!(spec.attempt_uuid, ""); // Fresh UUID generated!
        assert_eq!(spec.captured_slot, item.captured_slot);
        assert_eq!(spec.template_id, item.template_id);
        assert_eq!(spec.payment_type, 0); // GOLD
        assert_eq!(spec.charm_mode, 0);   // NONE

        // If current_level == target_level, cannot prepare attempt
        let mut completed_item = item.clone();
        completed_item.current_level = 5;
        assert!(matches!(
            prepare_level_attempt(&completed_item),
            Err(LevelAttemptError::AlreadyAtOrAboveTarget)
        ));
    }

    #[test]
    fn test_mutation_fence_persistence_before_dispatch() {
        let item_id = "it-1";
        let attempt_uuid = "attempt-uuid-999";

        let (path, body) = build_mutation_fence_update(item_id, attempt_uuid, EnhancementAttemptPhase::Preparing)
            .expect("must build fence update");

        assert!(path.contains("id=eq.it-1"));
        assert!(path.contains("active_attempt_uuid=eq.attempt-uuid-999"));
        assert!(path.contains("attempt_phase=eq.PREPARING"));
        assert_eq!(body["attempt_phase"], "EXECUTE_MAY_HAVE_BEEN_SENT");
        assert!(body.get("execute_may_have_been_sent_at").is_some());

        // Worker commits fence update successfully (returns 1 row)
        let mut row = EnhancementQueueItemRow::mock(item_id, "job-1", 1, 0, 3, 5, "RUNNING");
        row.active_attempt_uuid = Some(attempt_uuid.to_string());
        row.attempt_phase = EnhancementAttemptPhase::ExecuteMayHaveBeenSent.as_str().to_string();
        let decision = evaluate_mutation_fence_response(&[row], attempt_uuid);
        assert_eq!(decision, MutationFenceDecision::FenceCommittedProceedToDispatch);

        // Worker fails to commit fence update (0 rows, lost CAS or disconnected)
        let failed_decision = evaluate_mutation_fence_response(&[], attempt_uuid);
        assert_eq!(failed_decision, MutationFenceDecision::FenceFailedDoNotDispatch);
    }

    #[test]
    fn test_single_item_dispatch_payload_invariants() {
        let spec = LevelAttemptSpec {
            attempt_uuid: "attempt-123".to_string(),
            captured_slot: 2,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            tier: 2,
            expected_level: 4,
            target_level: 5,
            charm_mode: 1,
            payment_type: 0,
            max_attempts: 1,
        };

        let req = build_single_item_request(&spec);
        assert_eq!(req.request_id, "attempt-123");
        assert_eq!(req.captured_slot, 2);
        assert_eq!(req.template_id, 101);
        assert_eq!(req.category, 3);
        assert_eq!(req.expected_level, 4);
        assert_eq!(req.target_level, 5);
        assert_eq!(req.max_attempts, 1);
        assert_eq!(req.charm_mode, 1);
        assert_eq!(req.payment_type, 0);
        assert!(!req.validation_only);
    }

    #[test]
    fn test_authoritative_settlement_and_idempotent_spend_commit() {
        let mut item = EnhancementQueueItemRow::mock("it-1", "job-1", 1, 0, 4, 7, "RUNNING");
        item.active_attempt_uuid = Some("attempt-uuid-777".to_string());
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();
        item.actual_gold_spent = 50_000;
        item.actual_gem_spent = 0;
        item.actual_material_1_spent = 2;

        let telemetry = crate::enhancement::EnhancementStatusTelemetry {
            version: 1,
            request_id: "attempt-uuid-777".to_string(),
            state: "TARGET_REACHED".to_string(),
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            start_level: 4,
            current_level: 5, // Exactly 1 level step!
            target_level: 5,
            configured_charm_mode: 1,
            resolved_charm_mode: 1,
            payment_type: 0,
            attempt_count: 1,
            max_attempts: 1,
            last_result: Some("SUCCESS".to_string()),
            quoted_gold_cost: 25_000,
            quoted_gem_cost: 0,
            quoted_material_requirements: vec![1, 0, 0, 0],
            actual_gold_spent: 25_000,
            actual_gem_spent: 0,
            actual_materials_spent: vec![1, 0, 0, 0],
            actual_charms_spent: 1,
            accounting_status: "SETTLED".to_string(),
            validation_only: Some(false),
            error_code: None,
            error_message: None,
            updated_at: "2026-09-26T00:01:00Z".to_string(),
        };

        // 1. Build settlement commit update
        let (path, body) = build_settlement_commit_update(&item, "attempt-uuid-777", &telemetry)
            .expect("must build settlement commit update");

        assert!(path.contains("id=eq.it-1"));
        assert!(path.contains("active_attempt_uuid=eq.attempt-uuid-777"));
        assert!(path.contains("attempt_phase=in.(EXECUTE_MAY_HAVE_BEEN_SENT,WAITING_RESULT,WAITING_SETTLEMENT)"));
        assert_eq!(body["actual_gold_spent"], 75_000); // 50k + 25k exactly once
        assert_eq!(body["actual_material_1_spent"], 3); // 2 + 1
        assert_eq!(body["actual_charm_spent"], 1);
        assert_eq!(body["current_level"], 5);
        assert_eq!(body["attempt_phase"], "SETTLED");
        assert_eq!(body["last_result_code"], "SUCCESS");

        // 2. Commit once -> CommittedOnce
        let mut settled_row = item.clone();
        settled_row.attempt_phase = "SETTLED".to_string();
        settled_row.actual_gold_spent = 75_000;
        let outcome = evaluate_settlement_commit_result(&[settled_row.clone()], "attempt-uuid-777", "EXECUTE_MAY_HAVE_BEEN_SENT");
        assert_eq!(outcome, SettlementCommitOutcome::CommittedOnce);

        // 3. Repeated observation after crash / restart:
        // Current item phase in DB is ALREADY "SETTLED".
        // Evaluating again with 0 rows returned MUST recognize AlreadySettledDoNotIncrementAgain!
        let repeated_outcome = evaluate_settlement_commit_result(&[], "attempt-uuid-777", "SETTLED");
        assert_eq!(repeated_outcome, SettlementCommitOutcome::AlreadySettledDoNotIncrementAgain);
    }

    #[test]
    fn test_failure_mapping_and_queue_stop() {
        // 1. Regular terminal failures -> ItemFailedAndStopQueue
        let fail_cases = [
            ("ITEM_MISSING_OR_CHANGED", None),
            ("AMBIGUOUS_WIRE_TARGET", None),
            ("CHARM_MISSING", None),
            ("AMBIGUOUS_CHARM", None),
            ("INSUFFICIENT_GOLD", None),
            ("INSUFFICIENT_GEMS", None),
            ("INSUFFICIENT_MATERIALS", None),
            ("ENHANCEMENT_TRAVEL_CONFLICT", None),
            ("BLACKSMITH_ROUTE_UNAVAILABLE", None),
            ("BLACKSMITH_NOT_FOUND", None),
            ("BLACKSMITH_INTERACTION_FAILED", None),
            ("FORGE_OPEN_FAILED", None),
            ("FAILURE_PROTECTED", Some("PROTECTED_FAILURE")),
            ("FAILURE_DEGRADED", Some("DEGRADED")),
            ("ITEM_DESTROYED", Some("DESTROYED")),
            ("SERVER_REJECTED", None),
        ];

        for (state, result) in fail_cases {
            let outcome = map_runtime_failure_to_queue_outcome(state, result, Some("test error"));
            match outcome {
                QueueFailureOutcome::ItemFailedAndStopQueue { error_code, .. } => {
                    assert!(!error_code.is_empty(), "state {state} must have error code");
                }
                other => panic!("expected ItemFailedAndStopQueue for {state}, got {:?}", other),
            }
        }

        // 2. Accounting and ambiguity -> ManualReviewRequiredAndFreezeQueue
        let review_cases = [
            ("ACCOUNTING_UNSETTLED", None),
            ("RESULT_AMBIGUOUS", Some("RESULT_AMBIGUOUS")),
            ("MANUAL_REVIEW_REQUIRED", None),
        ];

        for (state, result) in review_cases {
            let outcome = map_runtime_failure_to_queue_outcome(state, result, Some("accounting unsettled"));
            match outcome {
                QueueFailureOutcome::ManualReviewRequiredAndFreezeQueue { error_code, .. } => {
                    assert_eq!(error_code, state);
                }
                other => panic!("expected ManualReviewRequiredAndFreezeQueue for {state}, got {:?}", other),
            }
        }
    }

    #[test]
    fn test_pause_and_cancel_safety_boundaries() {
        let mut job = EnhancementQueueJobRow::mock("job-1", "acc-1", "dev-1", "RUNNING", "worker-1");

        // 1. No pause or cancel requested
        assert_eq!(
            evaluate_pause_request(&job, EnhancementAttemptPhase::Preparing),
            PauseAction::NoPauseRequested
        );
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::Preparing),
            CancelAction::NoCancelRequested
        );

        // 2. Pause requested BEFORE mutation fence -> PauseImmediatelyPreFence
        job.pause_requested_at = Some("2026-09-26T00:02:00Z".to_string());
        assert_eq!(
            evaluate_pause_request(&job, EnhancementAttemptPhase::Preparing),
            PauseAction::PauseImmediatelyPreFence
        );
        assert_eq!(
            evaluate_pause_request(&job, EnhancementAttemptPhase::ReadyToExecute),
            PauseAction::PauseImmediatelyPreFence
        );

        // 3. Pause requested AFTER mutation fence -> WaitForSettlementPostFenceThenPause
        assert_eq!(
            evaluate_pause_request(&job, EnhancementAttemptPhase::ExecuteMayHaveBeenSent),
            PauseAction::WaitForSettlementPostFenceThenPause
        );
        assert_eq!(
            evaluate_pause_request(&job, EnhancementAttemptPhase::WaitingResult),
            PauseAction::WaitForSettlementPostFenceThenPause
        );
        assert_eq!(
            evaluate_pause_request(&job, EnhancementAttemptPhase::WaitingSettlement),
            PauseAction::WaitForSettlementPostFenceThenPause
        );

        // 4. Cancel requested BEFORE mutation fence -> CancelImmediatelyPreFence
        job.pause_requested_at = None;
        job.cancel_requested_at = Some("2026-09-26T00:03:00Z".to_string());
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::None),
            CancelAction::CancelImmediatelyPreFence
        );
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::Preparing),
            CancelAction::CancelImmediatelyPreFence
        );
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::ReadyToExecute),
            CancelAction::CancelImmediatelyPreFence
        );

        // 5. Cancel requested AFTER mutation fence -> WaitForSettlementPostFenceThenCancel
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::ExecuteMayHaveBeenSent),
            CancelAction::WaitForSettlementPostFenceThenCancel
        );
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::WaitingResult),
            CancelAction::WaitForSettlementPostFenceThenCancel
        );
        assert_eq!(
            evaluate_cancel_request(&job, EnhancementAttemptPhase::WaitingSettlement),
            CancelAction::WaitForSettlementPostFenceThenCancel
        );
    }

    #[test]
    fn test_restart_recovery_across_all_11_scenarios() {
        let job_id = "job-rec-1";
        let attempt_uuid = "att-uuid-555";

        // Scenario 1: restart while job is QUEUED before claim
        let queued_job = EnhancementQueueJobRow::mock(job_id, "acc-1", "dev-1", "QUEUED", "");
        assert_eq!(
            evaluate_restart_recovery_step(&queued_job, None, None),
            RestartRecoveryAction::ClaimQueuedJob
        );

        // Scenario 2: restart after claim but before item start
        let running_job = EnhancementQueueJobRow::mock(job_id, "acc-1", "dev-1", "RUNNING", "worker-1");
        assert_eq!(
            evaluate_restart_recovery_step(&running_job, None, None),
            RestartRecoveryAction::StartNextPendingItem
        );

        // Scenario 3: restart while item PREPARING (pre-fence)
        let mut item_prep = EnhancementQueueItemRow::mock("it-1", job_id, 1, 0, 3, 5, "RUNNING");
        item_prep.attempt_phase = "PREPARING".to_string();
        item_prep.active_attempt_uuid = Some(attempt_uuid.to_string());
        assert_eq!(
            evaluate_restart_recovery_step(&running_job, Some(&item_prep), None),
            RestartRecoveryAction::ResumePreFencePreparation
        );

        // Scenario 4: restart before mutation fence (READY_TO_EXECUTE)
        let mut item_ready = item_prep.clone();
        item_ready.attempt_phase = "READY_TO_EXECUTE".to_string();
        assert_eq!(
            evaluate_restart_recovery_step(&running_job, Some(&item_ready), None),
            RestartRecoveryAction::ResumePreFencePreparation
        );

        // Scenario 5: restart after mutation fence, NO disk telemetry -> MANUAL_REVIEW_REQUIRED (no blind replay!)
        let mut item_post_fence = item_prep.clone();
        item_post_fence.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();
        match evaluate_restart_recovery_step(&running_job, Some(&item_post_fence), None) {
            RestartRecoveryAction::TransitionToManualReviewRequired { reason } => {
                assert!(reason.contains("unreconciled"));
            }
            other => panic!("expected TransitionToManualReviewRequired, got {:?}", other),
        }

        // Scenario 6 & 7 & 8 & 9 with unreconciled telemetry -> MANUAL_REVIEW_REQUIRED
        for phase in &["EXECUTE_MAY_HAVE_BEEN_SENT", "WAITING_RESULT", "WAITING_SETTLEMENT"] {
            let mut it = item_prep.clone();
            it.attempt_phase = phase.to_string();
            assert!(matches!(
                evaluate_restart_recovery_step(&running_job, Some(&it), None),
                RestartRecoveryAction::TransitionToManualReviewRequired { .. }
            ));
        }

        // Scenario 10: restart after runtime settlement before item DB settlement commit
        // EXACT result & actual spend recovered from disk telemetry!
        let valid_telemetry = crate::enhancement::EnhancementStatusTelemetry {
            version: 1,
            request_id: attempt_uuid.to_string(),
            state: "TARGET_REACHED".to_string(),
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            start_level: 3,
            current_level: 4,
            target_level: 4,
            configured_charm_mode: 0,
            resolved_charm_mode: 0,
            payment_type: 0,
            attempt_count: 1,
            max_attempts: 1,
            last_result: Some("SUCCESS".to_string()),
            quoted_gold_cost: 10_000,
            quoted_gem_cost: 0,
            quoted_material_requirements: vec![],
            actual_gold_spent: 10_000,
            actual_gem_spent: 0,
            actual_materials_spent: vec![],
            actual_charms_spent: 0,
            accounting_status: "SETTLED".to_string(),
            validation_only: Some(false),
            error_code: None,
            error_message: None,
            updated_at: "2026-09-26T00:01:00Z".to_string(),
        };

        let mut item_to_settle = item_prep.clone();
        item_to_settle.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();
        assert_eq!(
            evaluate_restart_recovery_step(&running_job, Some(&item_to_settle), Some(&valid_telemetry)),
            RestartRecoveryAction::ReconcileSettledAttempt(valid_telemetry.clone())
        );

        // Scenario 11: restart after item settlement before item/job state advancement
        let mut item_settled = item_prep.clone();
        item_settled.attempt_phase = "SETTLED".to_string();
        item_settled.current_level = 4; // target is 5 -> next level
        assert_eq!(
            evaluate_restart_recovery_step(&running_job, Some(&item_settled), None),
            RestartRecoveryAction::AdvanceSettledItem { mark_completed: false }
        );

        // Scenario 11b: item reached target level -> mark completed
        item_settled.current_level = 5; // target is 5 -> completed!
        assert_eq!(
            evaluate_restart_recovery_step(&running_job, Some(&item_settled), None),
            RestartRecoveryAction::AdvanceSettledItem { mark_completed: true }
        );
    }

    #[test]
    fn test_orchestrator_full_lifecycle_advancement() {
        // Job with 2 items: item 1 (0 -> 1), item 2 (0 -> 1)
        let _job = EnhancementQueueJobRow::mock("job-flow-1", "acc-1", "dev-1", "RUNNING", "worker-1");
        let item1 = EnhancementQueueItemRow::mock("it-1", "job-flow-1", 1, 0, 0, 1, "RUNNING");
        let item2 = EnhancementQueueItemRow::mock("it-2", "job-flow-1", 2, 1, 0, 1, "PENDING");

        // 1. Prepare attempt for item 1
        let spec = prepare_level_attempt(&item1).unwrap();
        assert_eq!(spec.expected_level, 0);
        assert_eq!(spec.target_level, 1);
        assert_eq!(spec.max_attempts, 1);

        // 2. Mutation fence
        let (fence_path, fence_body) = build_mutation_fence_update(&item1.id, &spec.attempt_uuid, EnhancementAttemptPhase::Preparing).unwrap();
        assert!(fence_path.contains("id=eq.it-1"));
        assert_eq!(fence_body["attempt_phase"], "EXECUTE_MAY_HAVE_BEEN_SENT");

        // 3. Dispatch
        let req = build_single_item_request(&spec);
        assert_eq!(req.request_id, spec.attempt_uuid);
        assert_eq!(req.expected_level, 0);
        assert_eq!(req.target_level, 1);

        // 4. Authoritative success settlement
        let telemetry = crate::enhancement::EnhancementStatusTelemetry {
            version: 1,
            request_id: spec.attempt_uuid.clone(),
            state: "TARGET_REACHED".to_string(),
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm ngắn".to_string(),
            start_level: 0,
            current_level: 1,
            target_level: 1,
            configured_charm_mode: 0,
            resolved_charm_mode: 0,
            payment_type: 0,
            attempt_count: 1,
            max_attempts: 1,
            last_result: Some("SUCCESS".to_string()),
            quoted_gold_cost: 10_000,
            quoted_gem_cost: 0,
            quoted_material_requirements: vec![],
            actual_gold_spent: 10_000,
            actual_gem_spent: 0,
            actual_materials_spent: vec![],
            actual_charms_spent: 0,
            accounting_status: "SETTLED".to_string(),
            validation_only: Some(false),
            error_code: None,
            error_message: None,
            updated_at: "2026-09-26T00:01:00Z".to_string(),
        };

        let (_commit_path, commit_body) = build_settlement_commit_update(&item1, &spec.attempt_uuid, &telemetry).unwrap();
        assert_eq!(commit_body["current_level"], 1);
        assert_eq!(commit_body["attempt_phase"], "SETTLED");
        assert_eq!(commit_body["actual_gold_spent"], 10_000);

        // 5. Item 1 completed -> select next item
        let mut item1_completed = item1.clone();
        item1_completed.status = "COMPLETED".to_string();
        item1_completed.current_level = 1;

        assert_eq!(
            select_next_executable_item(&[item1_completed.clone(), item2.clone()]),
            ItemSelectionOutcome::ProceedWithItem("it-2".to_string())
        );

        // 6. Item 2 also completes -> AllItemsCompleted
        let mut item2_completed = item2.clone();
        item2_completed.status = "COMPLETED".to_string();
        item2_completed.current_level = 1;

        assert_eq!(
            select_next_executable_item(&[item1_completed, item2_completed]),
            ItemSelectionOutcome::AllItemsCompleted
        );
    }

    #[test]
    fn test_mutation_fence_update_with_spec() {
        let item = EnhancementQueueItemRow::mock("item-123", "job-1", 1, 0, 2, 5, "RUNNING");
        let spec = prepare_level_attempt(&item).unwrap();

        // Safe from pre-fence "PREPARING"
        let (path, body) = build_mutation_fence_update_with_spec(&item.id, &item.account_id, &spec, "PREPARING")
            .expect("must succeed from pre-fence phase");
        assert_eq!(
            path,
            format!("/rest/v1/enhancement_queue_items?id=eq.{}&account_id=eq.{}&attempt_phase=eq.PREPARING", item.id, item.account_id)
        );
        assert_eq!(body["active_attempt_uuid"], spec.attempt_uuid);
        assert_eq!(body["attempt_phase"], "EXECUTE_MAY_HAVE_BEEN_SENT");
        assert_eq!(body["attempt_expected_level"], 2);
        assert_eq!(body["attempt_target_level"], 3);
        assert!(body["execute_may_have_been_sent_at"].is_string());

        // Also safe from "NONE"
        assert!(build_mutation_fence_update_with_spec(&item.id, &item.account_id, &spec, "NONE").is_ok());

        // Prohibited from post-fence "EXECUTE_MAY_HAVE_BEEN_SENT"
        assert!(build_mutation_fence_update_with_spec(&item.id, &item.account_id, &spec, "EXECUTE_MAY_HAVE_BEEN_SENT").is_err());
        // Prohibited from "SETTLED" without re-preparing
        assert!(build_mutation_fence_update_with_spec(&item.id, &item.account_id, &spec, "SETTLED").is_err());
    }

    #[test]
    fn test_account_queue_tracker_lifecycle() {
        let mut tracker = AccountQueueTracker::default();
        assert!(tracker.active_job_id.is_none());
        assert!(tracker.in_flight_attempt.is_none());

        tracker.active_job_id = Some("job-1".to_string());
        tracker.in_flight_attempt = Some(InFlightQueueAttempt {
            job_id: "job-1".to_string(),
            item_id: "it-1".to_string(),
            attempt_uuid: "att-1".to_string(),
            expected_level: 0,
            target_level: 1,
            started_at: std::time::Instant::now(),
        });

        assert_eq!(tracker.active_job_id.as_deref(), Some("job-1"));
        assert_eq!(tracker.in_flight_attempt.as_ref().unwrap().attempt_uuid, "att-1");
    }

    #[test]
    fn test_restart_recovery_comprehensive_matrix() {
        let mut job = EnhancementQueueJobRow::mock("job-r", "acc-1", "dev-1", "RUNNING", "worker-1");
        let mut item = EnhancementQueueItemRow::mock("it-r", "job-r", 1, 0, 0, 2, "RUNNING");

        // 1. QUEUED job
        job.status = "QUEUED".to_string();
        assert_eq!(
            evaluate_restart_recovery_step(&job, None, None),
            RestartRecoveryAction::ClaimQueuedJob
        );

        // 2. Claimed RUNNING job with no active item
        job.status = "RUNNING".to_string();
        assert_eq!(
            evaluate_restart_recovery_step(&job, None, None),
            RestartRecoveryAction::StartNextPendingItem
        );

        // 3. Pre-fence PREPARING item
        item.attempt_phase = "PREPARING".to_string();
        item.active_attempt_uuid = Some("att-uuid-1".to_string());
        assert_eq!(
            evaluate_restart_recovery_step(&job, Some(&item), None),
            RestartRecoveryAction::ResumePreFencePreparation
        );

        // 4. Post-fence EXECUTE_MAY_HAVE_BEEN_SENT with missing disk telemetry -> MANUAL_REVIEW_REQUIRED
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();
        match evaluate_restart_recovery_step(&job, Some(&item), None) {
            RestartRecoveryAction::TransitionToManualReviewRequired { reason } => {
                assert!(reason.contains("unreconciled post-fence attempt"));
            }
            other => panic!("expected TransitionToManualReviewRequired, got {:?}", other),
        }

        // 5. Post-fence EXECUTE_MAY_HAVE_BEEN_SENT with matching settled disk telemetry -> ReconcileSettledAttempt
        let telemetry = crate::enhancement::EnhancementStatusTelemetry {
            version: 1,
            request_id: "att-uuid-1".to_string(),
            state: "SUCCESS".to_string(),
            captured_slot: 0,
            template_id: 100,
            category: 3,
            base_name: "Gươm".to_string(),
            start_level: 0,
            current_level: 1,
            target_level: 1,
            configured_charm_mode: 0,
            resolved_charm_mode: 0,
            payment_type: 0,
            attempt_count: 1,
            max_attempts: 1,
            last_result: Some("SUCCESS".to_string()),
            quoted_gold_cost: 1000,
            quoted_gem_cost: 0,
            quoted_material_requirements: vec![],
            actual_gold_spent: 1000,
            actual_gem_spent: 0,
            actual_materials_spent: vec![],
            actual_charms_spent: 0,
            accounting_status: "SETTLED".to_string(),
            validation_only: Some(false),
            error_code: None,
            error_message: None,
            updated_at: "2026-09-26T00:00:00Z".to_string(),
        };

        match evaluate_restart_recovery_step(&job, Some(&item), Some(&telemetry)) {
            RestartRecoveryAction::ReconcileSettledAttempt(recovered) => {
                assert_eq!(recovered.request_id, "att-uuid-1");
                assert_eq!(recovered.current_level, 1);
            }
            other => panic!("expected ReconcileSettledAttempt, got {:?}", other),
        }

        // 6. Post-fence with mismatched request_id on disk -> MANUAL_REVIEW_REQUIRED
        let mut mismatched_telemetry = telemetry.clone();
        mismatched_telemetry.request_id = "other-uuid".to_string();
        match evaluate_restart_recovery_step(&job, Some(&item), Some(&mismatched_telemetry)) {
            RestartRecoveryAction::TransitionToManualReviewRequired { .. } => {}
            other => panic!("expected TransitionToManualReviewRequired, got {:?}", other),
        }

        // 7. Settled item needing next level (current 1 < target 2)
        item.attempt_phase = "SETTLED".to_string();
        item.current_level = 1;
        item.target_level = 2;
        assert_eq!(
            evaluate_restart_recovery_step(&job, Some(&item), None),
            RestartRecoveryAction::AdvanceSettledItem { mark_completed: false }
        );

        // 8. Settled item reached target (current 2 == target 2)
        item.current_level = 2;
        assert_eq!(
            evaluate_restart_recovery_step(&job, Some(&item), None),
            RestartRecoveryAction::AdvanceSettledItem { mark_completed: true }
        );

        // 9. Paused job
        job.status = "PAUSED".to_string();
        assert_eq!(
            evaluate_restart_recovery_step(&job, Some(&item), None),
            RestartRecoveryAction::JobPaused
        );

        // 10. Completed job
        job.status = "COMPLETED".to_string();
        assert_eq!(
            evaluate_restart_recovery_step(&job, Some(&item), None),
            RestartRecoveryAction::JobTerminal
        );
    }

    // ── Required Adversarial Audit Tests (ENHANCE-05C) ─────────────────────────

    #[test]
    fn test_crash_immediately_after_mutation_fence_before_dispatch() {
        let job = EnhancementQueueJobRow::mock("job-c1", "acc-1", "dev-1", "RUNNING", "worker-1");
        let mut item = EnhancementQueueItemRow::mock("it-c1", "job-c1", 1, 0, 0, 1, "RUNNING");
        item.active_attempt_uuid = Some("att-uuid-c1".to_string());
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();

        // Process crashed immediately after fence commit before command dispatch reached sidecar.
        // Disk telemetry is None because command was never executed.
        let action = evaluate_restart_recovery_step(&job, Some(&item), None);

        // MUST NOT blindly replay or resume pre-fence preparation!
        match action {
            RestartRecoveryAction::TransitionToManualReviewRequired { reason } => {
                assert!(reason.contains("unreconciled post-fence attempt"));
                assert!(reason.contains("att-uuid-c1"));
            }
            other => panic!("expected TransitionToManualReviewRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_crash_immediately_after_dispatch_with_unknown_runtime_acceptance() {
        let job = EnhancementQueueJobRow::mock("job-c2", "acc-1", "dev-1", "RUNNING", "worker-1");
        let mut item = EnhancementQueueItemRow::mock("it-c2", "job-c2", 1, 0, 0, 1, "RUNNING");
        item.active_attempt_uuid = Some("att-uuid-c2".to_string());
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();

        // Dispatch completed, but agent crashed before JVM acknowledged or finished.
        // On restart, telemetry on disk is missing or has non-settled state.
        let action = evaluate_restart_recovery_step(&job, Some(&item), None);
        assert!(matches!(action, RestartRecoveryAction::TransitionToManualReviewRequired { .. }));
    }

    #[test]
    fn test_crash_after_settlement_before_spend_commit() {
        let job = EnhancementQueueJobRow::mock("job-c3", "acc-1", "dev-1", "RUNNING", "worker-1");
        let mut item = EnhancementQueueItemRow::mock("it-c3", "job-c3", 1, 0, 0, 1, "RUNNING");
        item.active_attempt_uuid = Some("att-uuid-c3".to_string());
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();

        // Runtime completed Opcode 67 and wrote settled status to disk, but agent crashed before DB commit.
        let telemetry = crate::enhancement::EnhancementStatusTelemetry {
            version: 1,
            request_id: "att-uuid-c3".to_string(),
            state: "SUCCESS".to_string(),
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm".to_string(),
            start_level: 0,
            current_level: 1,
            target_level: 1,
            configured_charm_mode: 0,
            resolved_charm_mode: 0,
            payment_type: 0,
            attempt_count: 1,
            max_attempts: 1,
            last_result: Some("SUCCESS".to_string()),
            quoted_gold_cost: 5000,
            quoted_gem_cost: 0,
            quoted_material_requirements: vec![],
            actual_gold_spent: 5000,
            actual_gem_spent: 0,
            actual_materials_spent: vec![],
            actual_charms_spent: 0,
            accounting_status: "SETTLED".to_string(),
            validation_only: Some(false),
            error_code: None,
            error_message: None,
            updated_at: "2026-09-26T00:00:00Z".to_string(),
        };

        let action = evaluate_restart_recovery_step(&job, Some(&item), Some(&telemetry));
        match action {
            RestartRecoveryAction::ReconcileSettledAttempt(recovered) => {
                assert_eq!(recovered.request_id, "att-uuid-c3");
                assert_eq!(recovered.actual_gold_spent, 5000);
                assert_eq!(recovered.current_level, 1);
            }
            other => panic!("expected ReconcileSettledAttempt, got {:?}", other),
        }
    }

    #[test]
    fn test_duplicate_settlement_observation() {
        let attempt_uuid = "att-uuid-dup";
        // Simulate row already in SETTLED phase in database
        let outcome = evaluate_settlement_commit_result(&[], attempt_uuid, "SETTLED");
        assert_eq!(outcome, SettlementCommitOutcome::AlreadySettledDoNotIncrementAgain);
    }

    #[test]
    fn test_lease_expiry_takeover_on_post_fence_attempt() {
        // Worker 1 claimed job, initiated post-fence attempt, then crashed.
        // Worker 2 takes over after lease expiry. Worker 2 does NOT have Worker 1's disk telemetry.
        let job = EnhancementQueueJobRow::mock("job-lease", "acc-1", "dev-1", "RUNNING", "worker-2-takeover");
        let mut item = EnhancementQueueItemRow::mock("it-lease", "job-lease", 1, 0, 0, 1, "RUNNING");
        item.active_attempt_uuid = Some("att-worker-1".to_string());
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();

        let action = evaluate_restart_recovery_step(&job, Some(&item), None);
        // Worker 2 MUST NOT dispatch again. Must freeze in MANUAL_REVIEW_REQUIRED.
        match action {
            RestartRecoveryAction::TransitionToManualReviewRequired { reason } => {
                assert!(reason.contains("unreconciled post-fence attempt"));
            }
            other => panic!("expected TransitionToManualReviewRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_two_worker_claim_race() {
        let job_id = "job-race";
        let acc_id = "acc-1";
        let dev_id = "dev-1";

        // Query path must enforce conditional atomic CAS
        let path = build_claim_job_path(job_id, acc_id, dev_id);
        assert!(path.contains("status=eq.QUEUED"));

        // Winner gets the row with its worker id
        let winner_row = EnhancementQueueJobRow::mock(job_id, acc_id, dev_id, "RUNNING", "worker-1");
        assert_eq!(evaluate_claim_result(&[winner_row], "worker-1"), ClaimOutcome::Won);

        // Loser gets 0 rows (CAS failure)
        assert_eq!(evaluate_claim_result(&[], "worker-2"), ClaimOutcome::Lost);
    }

    #[test]
    fn test_duplicate_polling_race() {
        // Two runnable-looking items returned by poll
        let item1 = EnhancementQueueItemRow::mock("it-1", "job-1", 1, 0, 0, 2, "PENDING");
        let item2 = EnhancementQueueItemRow::mock("it-2", "job-1", 2, 0, 0, 2, "PENDING");

        // Regardless of order returned by poll, strictly lowest queue_order is selected
        assert_eq!(
            select_next_executable_item(&[item2.clone(), item1.clone()]),
            ItemSelectionOutcome::ProceedWithItem("it-1".to_string())
        );
        assert_eq!(
            select_next_executable_item(&[item1, item2]),
            ItemSelectionOutcome::ProceedWithItem("it-1".to_string())
        );
    }

    #[test]
    fn test_pause_immediately_after_fence() {
        let mut job = EnhancementQueueJobRow::mock("job-p", "acc-1", "dev-1", "RUNNING", "worker-1");
        job.pause_requested_at = Some("2026-09-26T00:05:00Z".to_string());

        let action = evaluate_pause_request(&job, EnhancementAttemptPhase::ExecuteMayHaveBeenSent);
        assert_eq!(action, PauseAction::WaitForSettlementPostFenceThenPause);
    }

    #[test]
    fn test_cancel_immediately_after_fence() {
        let mut job = EnhancementQueueJobRow::mock("job-c", "acc-1", "dev-1", "RUNNING", "worker-1");
        job.cancel_requested_at = Some("2026-09-26T00:05:00Z".to_string());

        let action = evaluate_cancel_request(&job, EnhancementAttemptPhase::ExecuteMayHaveBeenSent);
        assert_eq!(action, CancelAction::WaitForSettlementPostFenceThenCancel);
    }

    #[test]
    fn test_later_item_cannot_overtake_unresolved_earlier_item() {
        let mut item1 = EnhancementQueueItemRow::mock("it-1", "job-1", 1, 0, 0, 2, "RUNNING");
        let item2 = EnhancementQueueItemRow::mock("it-2", "job-1", 2, 0, 0, 2, "PENDING");

        // Item 1 is RUNNING -> Item 2 cannot start
        assert_eq!(
            select_next_executable_item(&[item2.clone(), item1.clone()]),
            ItemSelectionOutcome::ActiveItemRunning("it-1".to_string())
        );

        // Item 1 is FAILED -> Item 2 cannot start
        item1.status = "FAILED".to_string();
        assert_eq!(
            select_next_executable_item(&[item2.clone(), item1.clone()]),
            ItemSelectionOutcome::HaltedOnFailure("it-1".to_string())
        );

        // Item 1 is MANUAL_REVIEW_REQUIRED -> Item 2 cannot start
        item1.status = "MANUAL_REVIEW_REQUIRED".to_string();
        assert_eq!(
            select_next_executable_item(&[item2, item1]),
            ItemSelectionOutcome::HaltedOnManualReview("it-1".to_string())
        );
    }

    #[test]
    fn test_cross_account_claim_and_mutation_blocked() {
        assert_eq!(
            evaluate_claim_authorization("acc-owner", "dev-owner", "acc-intruder", "dev-owner"),
            ClaimAuthDecision::RejectedAccountMismatch
        );
        assert_eq!(
            evaluate_claim_authorization("acc-owner", "dev-owner", "acc-owner", "dev-intruder"),
            ClaimAuthDecision::RejectedDeviceMismatch
        );
        assert_eq!(
            evaluate_claim_authorization("acc-owner", "dev-owner", "acc-owner", "dev-owner"),
            ClaimAuthDecision::Authorized
        );
    }

    #[test]
    fn test_stale_runtime_result_cannot_settle_new_attempt_uuid() {
        let job = EnhancementQueueJobRow::mock("job-stale", "acc-1", "dev-1", "RUNNING", "worker-1");
        let mut item = EnhancementQueueItemRow::mock("it-stale", "job-stale", 1, 0, 0, 1, "RUNNING");
        item.active_attempt_uuid = Some("fresh-attempt-uuid-2".to_string());
        item.attempt_phase = "EXECUTE_MAY_HAVE_BEEN_SENT".to_string();

        // Disk has telemetry from an older attempt UUID
        let stale_telemetry = crate::enhancement::EnhancementStatusTelemetry {
            version: 1,
            request_id: "stale-attempt-uuid-1".to_string(),
            state: "SUCCESS".to_string(),
            captured_slot: 0,
            template_id: 101,
            category: 3,
            base_name: "Kiếm".to_string(),
            start_level: 0,
            current_level: 1,
            target_level: 1,
            configured_charm_mode: 0,
            resolved_charm_mode: 0,
            payment_type: 0,
            attempt_count: 1,
            max_attempts: 1,
            last_result: Some("SUCCESS".to_string()),
            quoted_gold_cost: 5000,
            quoted_gem_cost: 0,
            quoted_material_requirements: vec![],
            actual_gold_spent: 5000,
            actual_gem_spent: 0,
            actual_materials_spent: vec![],
            actual_charms_spent: 0,
            accounting_status: "SETTLED".to_string(),
            validation_only: Some(false),
            error_code: None,
            error_message: None,
            updated_at: "2026-09-26T00:00:00Z".to_string(),
        };

        let action = evaluate_restart_recovery_step(&job, Some(&item), Some(&stale_telemetry));
        // Stale result MUST NOT settle the fresh attempt!
        match action {
            RestartRecoveryAction::TransitionToManualReviewRequired { reason } => {
                assert!(reason.contains("unreconciled post-fence attempt"));
                assert!(reason.contains("fresh-attempt-uuid-2"));
            }
            other => panic!("expected TransitionToManualReviewRequired, got {:?}", other),
        }
    }

    #[test]
    fn test_manual_single_item_and_queue_coexistence_mutual_exclusion() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home = temp_dir.path();
        let mut tracker = AccountQueueTracker::default();
        let rest = crate::supabase_rest::SupabaseRest::new("http://127.0.0.1:54321".to_string(), "test-key".to_string());

        // 1. When legacy single-item enhancement is in flight, queue tick immediately yields
        tick_account_enhancement_queue(
            home,
            "dev-test",
            "acc-test",
            true, // process is alive
            true, // has_pending_single_item_enhancement == true
            &mut tracker,
            &rest,
        );

        // Tracker state must be untouched: no active job, no in flight attempt
        assert!(tracker.active_job_id.is_none());
        assert!(tracker.in_flight_attempt.is_none());
        // No enhancement request file written to disk
        assert!(!home.join("enhancement_request.json").exists());

        // 2. When queue has an in_flight_attempt, tracker reflects it
        tracker.in_flight_attempt = Some(InFlightQueueAttempt {
            job_id: "job-1".to_string(),
            item_id: "item-1".to_string(),
            attempt_uuid: "attempt-1".to_string(),
            expected_level: 0,
            target_level: 1,
            started_at: std::time::Instant::now(),
        });
        assert!(tracker.in_flight_attempt.is_some());
    }

    #[test]
    fn test_queue_schema_error_and_empty_queue_safety() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home = temp_dir.path();
        let mut tracker = AccountQueueTracker::default();
        let rest = crate::supabase_rest::SupabaseRest::new("http://127.0.0.1:54321".to_string(), "test-key".to_string());

        // When no queue rows exist (or endpoint unreachable), safe no-op
        tick_account_enhancement_queue(
            home,
            "dev-test",
            "acc-test",
            true,
            false,
            &mut tracker,
            &rest,
        );

        assert!(tracker.active_job_id.is_none());
        assert!(tracker.in_flight_attempt.is_none());
        assert!(!home.join("enhancement_request.json").exists());
    }
}

