mod actor;
mod affect;
mod belief;
mod config;
mod delta;
mod directive;
mod interest;
mod relationship;
mod traits;

pub use actor::{ActorState, GrowthEvent};
pub use affect::AffectState;
pub use belief::Belief;
pub use config::{GrowthConfig, GrowthRate, ProactivityConfig, QuietHoursUtc};
pub use delta::{
    AffectShift, BeliefChange, Delta, RelationshipChange, RelationshipInteraction,
    RelationshipSignalUpdate, TraitNudge,
};
pub use directive::{BehaviorDirective, DirectiveScope};
pub use interest::Interest;
pub use relationship::{Authority, ProactiveConsent, Relationship};
pub use traits::CoreTraits;
