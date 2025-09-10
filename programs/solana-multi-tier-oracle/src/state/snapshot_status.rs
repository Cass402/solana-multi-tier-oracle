
/// Comprehensive snapshot quality assessment for oracle data integrity validation.
/// 
/// # Oracle Data Quality Framework
/// 
/// This enum represents the result of analyzing historical oracle snapshots to determine
/// data quality and reliability for critical operations like redemptions or liquidations.
/// The framework goes beyond simple count validation to assess temporal distribution
/// patterns that could indicate manipulation, system failures, or irregular update behavior.
/// 
/// # Multi-Dimensional Quality Assessment
/// 
/// Rather than a binary sufficient/insufficient classification, this system evaluates:
/// 1. **Quantity**: Sufficient snapshot count for statistical reliability
/// 2. **Temporal Coverage**: Adequate time span to prevent gaming through selective timing
/// 3. **Distribution Quality**: Even temporal distribution to avoid clustering attacks
/// 
/// # Attack Vector Prevention
/// 
/// The granular failure modes prevent several attack scenarios:
/// - **Snapshot Stuffing**: Rapid snapshot generation to meet count requirements
/// - **Temporal Gaming**: Clustering updates during favorable market conditions
/// - **Selective Reporting**: Meeting minimum counts while avoiding unfavorable periods
#[derive(Clone, Debug, PartialEq)]
pub enum SnapshotStatus {
    /// Snapshots meet all quality requirements for reliable oracle operations.
    /// 
    /// # Quality Validation Criteria
    /// 
    /// This state indicates that historical snapshots demonstrate:
    /// - Sufficient quantity for statistical significance
    /// - Adequate temporal coverage spanning meaningful time periods
    /// - Proper distribution preventing manipulation through clustering
    /// 
    /// # Usage in Critical Operations
    /// 
    /// Only when snapshots achieve this status should they be used for high-stakes
    /// operations like collateral liquidations or protocol redemptions where
    /// data quality directly impacts financial outcomes.
    Sufficient {
        /// Total snapshots meeting quality criteria, indicating statistical reliability.
        /// Higher counts provide greater confidence in oracle stability and consistency.
        snapshot_count: u16,
        
        /// Time span covered by snapshots in hours, ensuring temporal breadth.
        /// Longer spans reduce the impact of short-term market anomalies or manipulation attempts.
        time_span_hours: u16,
        
        /// Maximum snapshots found in any single hour window, indicating distribution quality.
        /// Lower values suggest more even temporal distribution and reduced clustering risk.
        max_hourly_density: u16,
    },
    
    /// Insufficient snapshot count within the evaluation window.
    /// 
    /// # Reliability Implications
    /// 
    /// Indicates the oracle hasn't been updated frequently enough to provide reliable
    /// data for critical operations. This could result from network issues, validator
    /// problems, or insufficient economic incentives for oracle maintenance.
    /// 
    /// # Risk Mitigation Strategy
    /// 
    /// Operations requiring high data confidence should be delayed or rejected until
    /// sufficient snapshots accumulate, preventing decisions based on sparse data.
    InsufficientCount {
        /// Number of snapshots found (below minimum threshold).
        /// Provides transparency about how far short the count falls from requirements.
        found: u16,
        
        /// Minimum required snapshots for redemption eligibility.
        /// Establishes clear threshold for when oracle data becomes usable.
        required: u16,
    },
    
    /// Snapshots don't span sufficient time range, indicating temporal clustering.
    /// 
    /// # Temporal Gaming Prevention
    /// 
    /// May occur during periods of irregular update patterns or when updates are
    /// concentrated in specific time windows. This clustering could enable gaming
    /// where oracle updates are timed to coincide with favorable market conditions
    /// while avoiding periods of unfavorable price movements.
    /// 
    /// # Quality Assurance Rationale
    /// 
    /// Requiring adequate time span ensures that oracle data represents diverse
    /// market conditions rather than cherry-picked time periods that could
    /// misrepresent true market dynamics.
    InsufficientTimeSpan {
        /// Actual time span covered by snapshots in hours.
        /// Shows the gap between actual coverage and quality requirements.
        span_hours: u16,
        
        /// Minimum required time span for quality assurance.
        /// Defines the temporal breadth needed for reliable oracle data.
        required_hours: u16,
    },
    
    /// Too many snapshots clustered within hour windows, suggesting irregular patterns.
    /// 
    /// # Clustering Attack Detection
    /// 
    /// Could indicate manipulation attempts where an attacker rapidly generates
    /// multiple snapshots during favorable conditions to meet count requirements
    /// while avoiding unfavorable periods. This pattern undermines the statistical
    /// validity of using snapshot data for critical decisions.
    /// 
    /// # System Health Indicator
    /// 
    /// May also indicate technical issues like timestamp irregularities, system
    /// clock problems, or bulk data updates that compromise the organic nature
    /// of oracle data collection.
    ExcessiveClustering {
        /// Maximum snapshots found in any single hour.
        /// Quantifies the severity of clustering to enable risk assessment.
        max_per_hour: u16,
        
        /// Maximum allowed snapshots per hour window.
        /// Defines the threshold beyond which clustering becomes concerning.
        limit_per_hour: u16,
    },
    
    /// No snapshots exist within the evaluation window.
    /// 
    /// # System Inactivity Indicator
    /// 
    /// Indicates oracle has been inactive or data has been cleared, representing
    /// the most severe data quality failure. This state requires immediate attention
    /// as it suggests complete system failure or intentional data deletion.
    /// 
    /// # Fail-Safe Behavior
    /// 
    /// All critical operations should be prohibited in this state to prevent
    /// decisions based on completely absent data.
    NoSnapshots,
}

impl SnapshotStatus {
    /// Fast boolean check for snapshot sufficiency with zero-cost abstraction.
    /// 
    /// # Performance Optimization
    /// 
    /// Uses pattern matching with the matches! macro for efficient enum variant checking
    /// without the overhead of extracting field values. This enables frequent validation
    /// checks in hot code paths without performance penalties.
    /// 
    /// # Usage Pattern
    /// 
    /// Designed for guard clauses and early validation where only the sufficiency
    /// status matters, not the specific failure reasons or metadata values.
    #[inline(always)]
    pub fn is_sufficient(&self) -> bool {
        matches!(self, SnapshotStatus::Sufficient { .. })
    }
    
    /// Extract snapshot count with defensive handling of incomplete data variants.
    /// 
    /// # Data Availability Strategy
    /// 
    /// Not all enum variants contain snapshot count information due to different
    /// failure modes. This method provides a consistent interface while handling
    /// cases where count data isn't meaningful or available.
    /// 
    /// # Return Value Semantics
    /// 
    /// Returns 0 for variants where snapshot count isn't provided or meaningful,
    /// establishing a conservative default that prevents false confidence in
    /// data quality assessment.
    pub fn snapshot_count(&self) -> u16 {
        match self {
            // Direct count available for successful and insufficient count cases
            SnapshotStatus::Sufficient { snapshot_count, .. } => *snapshot_count,
            SnapshotStatus::InsufficientCount { found, .. } => *found,
            
            // Count not provided for time span failures - may have sufficient count
            // but poor temporal distribution, so returning 0 prevents misinterpretation
            SnapshotStatus::InsufficientTimeSpan { .. } => 0,
            
            // Count not provided for clustering failures - prevents using count
            // information when the distribution quality is compromised
            SnapshotStatus::ExcessiveClustering { .. } => 0,
            
            // No snapshots exist - count is definitively zero
            SnapshotStatus::NoSnapshots => 0,
        }
    }
}
