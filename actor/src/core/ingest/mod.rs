mod adoption;
mod identity;
mod normalize;
mod observation;

pub(super) use adoption::advance_adoption_ritual;
pub(super) use normalize::resolve_person;
pub(super) use observation::observe_inbound;
