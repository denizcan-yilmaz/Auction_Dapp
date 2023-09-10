use candid::{CandidType, Decode, Deserialize, Encode, Principal};
use ic_cdk::{caller, query, update};
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    BoundedStorable, DefaultMemoryImpl, StableBTreeMap, StableCell, Storable,
};
use std::{borrow::Cow, cell::RefCell, collections::HashMap};

type Memory = VirtualMemory<DefaultMemoryImpl>;

#[derive(Deserialize, CandidType)]
struct ItemBase {
    description: String,
    result_date: u64, //specifies when the auction will be closed automatically for the given item
    is_active: bool,
    latest_update: u64,
}

#[derive(Deserialize, CandidType)]
struct BidBase {
    bid_amount: u64,
    bid_date: u64, //kept as a unix timestamp
}

#[derive(Deserialize, CandidType)]
struct Bid {
    item_id: u64,
    bidder_principal: Principal,
    bid_date: u64, // will be kept as a unix timestamp
    bid_amount: u64,
}

#[derive(Deserialize, CandidType)]
struct Item {
    item_owner: Principal,
    id: u64,
    description: String,
    highest_bid: u64,
    latest_update: u64,
    result_date: u64,
    bid_vector: Vec<Bid>,
    is_active: bool,
}

impl Storable for Item {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for Item {
    const MAX_SIZE: u32 = 10_000;
    const IS_FIXED_SIZE: bool = false;
}

thread_local! {
    static MEMORY_MANAGER : RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    static ITEM_MAP: RefCell<StableBTreeMap<u64, Item, Memory>> = RefCell::new(StableBTreeMap::init(MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(0)))));

    //keeps track of the last used id, could be implemented by generating a random id as well, but wanted to try this out
    static ID_COUNTER: RefCell<StableCell<u64, Memory>> = RefCell::new(StableCell::init(
        MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(1))),
        u64::default()).unwrap());
}

fn get_and_inc_current_id() -> u64 {
    let mut id_tmp = 0;
    ID_COUNTER.with(|counter| {
        id_tmp = *(counter.borrow()).get();
        counter.borrow_mut().set(id_tmp + 1).unwrap();
    });
    return id_tmp;
}

#[query(name = "getAllItems")]
fn get_all_items() -> Option<HashMap<u64, Item>> {
    let mut map: HashMap<u64, Item> = HashMap::new();

    ITEM_MAP.with(|p| {
        for (k, v) in p.borrow().iter() {
            map.insert(k, v);
        }
    });
    return Some(map);
}

#[query(name = "getItem")]
fn get_item(key: u64) -> Option<Item> {
    ITEM_MAP.with(|p| p.borrow().get(&key))
}

#[update(name = "listItem")]
fn list_item(item: ItemBase) -> Option<Item> {
    let id_tmp = get_and_inc_current_id();

    let new_item: Item = Item {
        item_owner: caller(),
        id: id_tmp,
        description: item.description,
        highest_bid: 0,
        latest_update: item.latest_update,
        result_date: item.result_date,
        bid_vector: vec![],
        is_active: item.is_active,
    };

    return ITEM_MAP.with(|item| item.borrow_mut().insert(id_tmp, new_item));
}

#[update(name = "editItem")]
fn edit_item(key: u64, new_item: ItemBase) -> Result<String, String> {
    let mut ret_item: Option<Item> = None;
    let mut is_authorized: bool = true;

    ITEM_MAP.with(|items| {
        for (k, mut v) in items.borrow_mut().iter() {
            if k == key {
                if v.item_owner != caller() {
                    is_authorized = false;
                }
                v.description = new_item.description;
                v.result_date = new_item.result_date;
                v.is_active = new_item.is_active;
                v.latest_update = new_item.latest_update;
                ret_item = Some(v);
                break;
            }
        }
    });

    if !is_authorized {
        return Err("Item could not be edited. Most probably, could not be found".to_string());
    }
    match ret_item {
        Some(_) => {
            ITEM_MAP.with(|item| item.borrow_mut().insert(key, ret_item.unwrap()));
            Ok("Item edited successfully".to_string())
        }
        None => Err("Item could not be edited. Most probably, could not be found".to_string()),
    }
}

#[update(name = "stopListing")]
fn stop_listing(key: u64) -> Result<String, String> {
    let mut ret_item: Option<Item> = None;
    let mut is_authorized: bool = true;

    ITEM_MAP.with(|items| {
        for (k, mut v) in items.borrow_mut().iter() {
            if k == key {
                if v.item_owner != caller() {
                    is_authorized = false;
                }
                v.is_active = false;
                ret_item = Some(v);
                break;
            }
        }
    });

    if !is_authorized {
        return Err("You are not authorized to edit this item.".to_string());
    }
    match ret_item {
        Some(_) => {
            ITEM_MAP.with(|item| item.borrow_mut().insert(key, ret_item.unwrap()));
            Ok("Selected item  is no longer actively listed on the auction list.".to_string())
        }
        None => Err("Item could not be edited. Most probably, could not be found.".to_string()),
    }
}

#[update(name = "deleteItem")]
fn delete_item(key: u64) -> Result<String, String> {
    let mut found_item: Option<Item> = None;

    ITEM_MAP.with(|items| {
        found_item = items.borrow_mut().get(&key);
    });

    match found_item {
        Some(fi) => {
            if fi.item_owner != caller() {
                return Err(format!(
                    "You are not authorized to remove this item. The owner is: {}",
                    fi.item_owner
                ));
            }
            ITEM_MAP.with(|items| {
                items.borrow_mut().remove(&key);
            });
            Ok(format!("Item with id {} removed successfully", fi.id))
        }
        None => Err("Item could not be found.".to_string()),
    }
}

#[update(name = "bidForAnItem")]
fn bid_for_an_item(key: u64, bid: BidBase) -> Result<String, String> {
    let mut found_item: Option<Item> = None;

    ITEM_MAP.with(|items| {
        found_item = items.borrow_mut().get(&key);
    });

    match found_item {
        Some(fi) => {
            if fi.item_owner == caller() {
                return Err(format!("You cannot bid for you own item",));
            }
            if !fi.is_active {
                return Err(format!("The selected item is not actively listed.",));
            }
            if bid.bid_amount <= fi.highest_bid {
                return Err(format!(
                    "Your bid cannot be lower than the current highest bid.",
                ));
            }
            let fi_id = fi.id;
            let new_bid = Bid {
                item_id: key,
                bidder_principal: caller(),
                bid_amount: bid.bid_amount,
                bid_date: bid.bid_date,
            };
            let mut new_item = fi;
            new_item.highest_bid = new_bid.bid_amount;
            new_item.bid_vector.push(new_bid);
            ITEM_MAP.with(|items| items.borrow_mut().insert(key, new_item));
            Ok(format!("Successfully bidded for item {}", fi_id))
        }
        None => Err("Item could not be found.".to_string()),
    }
}
