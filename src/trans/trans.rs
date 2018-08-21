use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::sync::{Arc, RwLock};

use super::armor::{Arm, Armor, VolumeArmor};
use super::wal::Wal;
use super::{Eid, EntityType, Id, Txid};
use base::IntoRef;
use error::{Error, Result};
use volume::VolumeRef;

/// Cohort action in transaction
#[derive(Debug, Clone, Copy, Eq, PartialEq, Deserialize, Serialize)]
pub enum Action {
    New,
    Update,
    Delete,
}

/// Transable trait, be able to be added in transaction
pub trait Transable: Debug + Id + Send + Sync {
    fn action(&self) -> Action;
    fn commit(&mut self, vol: &VolumeRef) -> Result<()>;
    fn complete_commit(&mut self);
    fn abort(&mut self);
}

pub type TransableRef = Arc<RwLock<Transable>>;

/// Transaction
pub struct Trans {
    txid: Txid,
    cohorts: HashMap<Eid, TransableRef>,
    wal: Wal,
    wal_armor: VolumeArmor<Wal>,
}

impl Trans {
    pub fn new(txid: Txid, vol: &VolumeRef) -> Self {
        Trans {
            txid,
            cohorts: HashMap::new(),
            wal: Wal::new(txid),
            wal_armor: VolumeArmor::new(vol),
        }
    }

    #[inline]
    pub fn get_wal(&self) -> Wal {
        self.wal.clone()
    }

    #[inline]
    pub fn begin_trans(&mut self) -> Result<()> {
        self.wal_armor.save_item(&mut self.wal)
    }

    // add an entity to this transaction
    pub fn add_entity(
        &mut self,
        id: &Eid,
        entity: TransableRef,
        action: Action,
        ent_type: EntityType,
        arm: Arm,
    ) -> Result<()> {
        // add a wal entry and save wal
        self.wal.add_entry(id, action, ent_type, arm);
        self.wal_armor.save_item(&mut self.wal)?;

        // add entity to cohorts
        self.cohorts.entry(id.clone()).or_insert(entity);

        Ok(())
    }

    /// Commit transaction
    pub fn commit(&mut self, vol: &VolumeRef) -> Result<Wal> {
        //debug!("trans.commit: cohorts: {:#?}", self.cohorts);

        // commit each entity
        for entity in self.cohorts.values() {
            let mut ent = entity.write().unwrap();

            // make sure deleted entity is not in use
            if ent.action() == Action::Delete {
                let using_cnt = Arc::strong_count(&entity);
                if using_cnt > 1 {
                    error!(
                        "cannot delete entity in use (using: {})",
                        using_cnt,
                    );
                    return Err(Error::InUse);
                }
            }

            // commit entity
            ent.commit(&vol)?;
        }

        Ok(self.wal.clone())
    }

    // complete commit
    pub fn complete_commit(&mut self) {
        for entity in self.cohorts.values() {
            let mut ent = entity.write().unwrap();
            ent.complete_commit();
        }
        self.cohorts.clear();
        Txid::reset_current();
    }

    // abort transaction
    pub fn abort(&mut self, vol: &VolumeRef) -> Result<()> {
        // abort each entity
        for entity in self.cohorts.values() {
            let mut ent = entity.write().unwrap();
            ent.abort();
        }

        self.cohorts.clear();
        Txid::reset_current();

        // clean aborted entities
        self.wal.clean_aborted(vol)
    }
}

impl IntoRef for Trans {}

impl Debug for Trans {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Trans")
            .field("txid", &self.txid)
            .field("cohorts", &self.cohorts)
            .finish()
    }
}

/// Transaction reference type
pub type TransRef = Arc<RwLock<Trans>>;
