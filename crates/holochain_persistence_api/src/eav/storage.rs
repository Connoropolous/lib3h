use eav::{eavi::EntityAttributeValueIndex, query::EaviQuery, Attribute};
use error::PersistenceResult;
use objekt;
use std::{
    collections::BTreeSet,
    fmt::Debug,
    sync::{Arc, RwLock},
};

/// This provides a simple and flexible interface to define relationships between AddressableContent.
/// It does NOT provide storage for AddressableContent.
/// Use cas::storage::ContentAddressableStorage to store AddressableContent.
pub trait EntityAttributeValueStorage<A: Attribute>: objekt::Clone + Send + Sync + Debug {
    /// Adds the given EntityAttributeValue to the EntityAttributeValueStorage
    /// append only storage.
    fn add_eavi(
        &mut self,
        eav: &EntityAttributeValueIndex<A>,
    ) -> PersistenceResult<Option<EntityAttributeValueIndex<A>>>;

    /// Fetch the set of EntityAttributeValues that match constraints according to the latest hash version
    /// - None = no constraint
    /// - Some(Entity) = requires the given entity (e.g. all a/v pairs for the entity)
    /// - Some(Attribute) = requires the given attribute (e.g. all links)
    /// - Some(Value) = requires the given value (e.g. all entities referencing an Address)
    fn fetch_eavi(
        &self,
        query: &EaviQuery<A>,
    ) -> PersistenceResult<BTreeSet<EntityAttributeValueIndex<A>>>;

    // @TODO: would like to do this, but can't because of the generic type param
    // fn iter<I>(&self) -> I
    // where
    //     I: Iterator<Item = EntityAttributeValueIndex>;
}

clone_trait_object!(<A:Attribute>EntityAttributeValueStorage<A>);

#[derive(Clone, Debug, Default)]
pub struct ExampleEntityAttributeValueStorage<A: Attribute> {
    storage: Arc<RwLock<BTreeSet<EntityAttributeValueIndex<A>>>>,
}

impl<A: Attribute> ExampleEntityAttributeValueStorage<A> {
    pub fn new() -> ExampleEntityAttributeValueStorage<A>
    where
        A: std::default::Default,
    {
        Default::default()
    }
}

impl<A: Attribute> EntityAttributeValueStorage<A> for ExampleEntityAttributeValueStorage<A>
where
    A: std::marker::Send + std::marker::Sync,
{
    fn add_eavi(
        &mut self,
        eav: &EntityAttributeValueIndex<A>,
    ) -> PersistenceResult<Option<EntityAttributeValueIndex<A>>> {
        let mut map = self.storage.write()?;
        let new_eav = increment_key_till_no_collision(eav.clone(), map.clone())?;
        map.insert(new_eav.clone());
        Ok(Some(new_eav.clone()))
    }

    fn fetch_eavi(
        &self,
        query: &EaviQuery<A>,
    ) -> PersistenceResult<BTreeSet<EntityAttributeValueIndex<A>>> {
        let lock = self.storage.read()?;
        let set = (*lock).clone();
        let iter = set.iter().cloned();
        Ok(query.run(iter))
    }
}

impl<A: Attribute> PartialEq for EntityAttributeValueStorage<A> {
    fn eq(&self, other: &EntityAttributeValueStorage<A>) -> bool {
        let query = EaviQuery::default();
        self.fetch_eavi(&query) == other.fetch_eavi(&query)
    }
}

pub fn increment_key_till_no_collision<A: Attribute>(
    mut eav: EntityAttributeValueIndex<A>,
    map: BTreeSet<EntityAttributeValueIndex<A>>,
) -> PersistenceResult<EntityAttributeValueIndex<A>> {
    if map.iter().any(|e| e.index() == eav.index()) {
        let timestamp = eav.clone().index() + 1;
        eav.set_index(timestamp);
        increment_key_till_no_collision(eav, map)
    } else {
        Ok(eav)
    }
}
