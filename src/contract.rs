use cosmwasm_std::entry_point;
use cosmwasm_std::{to_binary, Binary, Deps, DepsMut, Env, MessageInfo, Order, Response, Storage};
use drand_verify::{derive_randomness, g1_from_variable, verify};

use crate::errors::ContractError;
use crate::msg::{
    ConfigResponse, ExecuteMsg, GetResponse, InstantiateMsg, LatestResponse, QueryMsg,
};
use crate::state::{beacons_storage, beacons_storage_read, config, config_read, Config};

use cw2::set_contract_version;

const CONTRACT_NAME: &str = "crates.io:rand";
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[entry_point]
pub fn instantiate(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    msg: InstantiateMsg,
) -> Result<Response, ContractError> {
    set_contract_version(deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
    config(deps.storage).save(&Config { pubkey: msg.pubkey })?;
    Ok(Response::default())
}

#[entry_point]
pub fn execute(
    deps: DepsMut,
    env: Env,
    info: MessageInfo,
    msg: ExecuteMsg,
) -> Result<Response, ContractError> {
    match msg {
        ExecuteMsg::Add {
            round,
            previous_signature,
            signature,
        } => try_add(deps, env, info, round, previous_signature, signature),
    }
}

pub fn try_add(
    deps: DepsMut,
    _env: Env,
    _info: MessageInfo,
    round: u64,
    previous_signature: Binary,
    signature: Binary,
) -> Result<Response, ContractError> {
    let Config { pubkey, .. } = config_read(deps.storage).load()?;
    let pk = g1_from_variable(&pubkey).map_err(|_| ContractError::InvalidPubkey {})?;
    let valid = verify(
        &pk,
        round,
        previous_signature.as_slice(),
        signature.as_slice(),
    )
    .unwrap_or(false);

    if !valid {
        return Err(ContractError::InvalidSignature {});
    }

    let randomness = derive_randomness(&signature);
    beacons_storage(deps.storage).set(&round.to_be_bytes(), &randomness);

    Ok(Response::new().add_attribute("randomness", Binary::from(randomness).to_base64()))
}

#[entry_point]
pub fn query(deps: Deps, _env: Env, msg: QueryMsg) -> Result<Binary, ContractError> {
    let response = match msg {
        QueryMsg::Config {} => to_binary(&query_config(deps)?)?,
        QueryMsg::Get { round } => to_binary(&query_get(deps, round)?)?,
        QueryMsg::Latest {} => to_binary(&query_latest(deps)?)?,
    };
    Ok(response)
}

fn query_config(deps: Deps) -> Result<ConfigResponse, ContractError> {
    let config = config_read(deps.storage).load()?;
    Ok(ConfigResponse {
        pubkey: config.pubkey,
    })
}

fn query_get(deps: Deps, round: u64) -> Result<GetResponse, ContractError> {
    let beacons = beacons_storage_read(deps.storage);
    let randomness = beacons.get(&round.to_be_bytes()).unwrap_or_default();
    Ok(GetResponse {
        randomness: randomness.into(),
    })
}

fn query_latest(deps: Deps) -> Result<LatestResponse, ContractError> {
    let store = beacons_storage_read(deps.storage);
    let mut iter = store.range(None, None, Order::Descending);
    let (key, value) = iter.next().ok_or(ContractError::NoBeacon {})?;

    Ok(LatestResponse {
        round: u64::from_be_bytes(Binary(key).to_array()?),
        randomness: value.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::{coins, from_binary};

    // $ node
    // > Uint8Array.from(Buffer.from("868f005eb8e6e4ca0a47c8a77ceaa5309a47978a7c71bc5cce96366b5d7a569937c529eeda66c7293784a9402801af31", "hex"))
    fn pubkey_loe_mainnet() -> Binary {
        vec![
            134, 143, 0, 94, 184, 230, 228, 202, 10, 71, 200, 167, 124, 234, 165, 48, 154, 71, 151,
            138, 124, 113, 188, 92, 206, 150, 54, 107, 93, 122, 86, 153, 55, 197, 41, 238, 218,
            102, 199, 41, 55, 132, 169, 64, 40, 1, 175, 49,
        ]
        .into()
    }

    #[test]
    fn proper_initialization() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &coins(1000, "earth"));
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };

        let res = instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();
        assert_eq!(res.messages.len(), 0);

        let response: ConfigResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Config {}).unwrap()).unwrap();
        assert_eq!(
            response,
            ConfigResponse {
                pubkey: pubkey_loe_mainnet(),
            }
        );
    }

    #[test]
    fn add_verifies_and_stores_randomness() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        let info = mock_info("anyone", &[]);
        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/72785
            round: 72785,
            previous_signature: hex::decode("a609e19a03c2fcc559e8dae14900aaefe517cb55c840f6e69bc8e4f66c8d18e8a609685d9917efbfb0c37f058c2de88f13d297c7e19e0ab24813079efe57a182554ff054c7638153f9b26a60e7111f71a0ff63d9571704905d3ca6df0b031747").unwrap().into(),
            signature: hex::decode("82f5d3d2de4db19d40a6980e8aa37842a0e55d1df06bd68bddc8d60002e8e959eb9cfa368b3c1b77d18f02a54fe047b80f0989315f83b12a74fd8679c4f12aae86eaf6ab5690b34f1fddd50ee3cc6f6cdf59e95526d5a5d82aaa84fa6f181e42").unwrap().into(),
        };
        execute(deps.as_mut(), mock_env(), info, msg).unwrap();

        let response: GetResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Get { round: 72785 }).unwrap())
                .unwrap();
        assert_eq!(
            response.randomness,
            hex::decode("8b676484b5fb1f37f9ec5c413d7d29883504e5b669f604a1ce68b3388e9ae3d9")
                .unwrap()
        );
    }

    #[test]
    fn add_fails_when_pubkey_is_invalid() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let mut broken: Vec<u8> = pubkey_loe_mainnet().into();
        broken.push(0xF9);
        let msg = InstantiateMsg {
            pubkey: broken.into(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        let info = mock_info("anyone", &[]);
        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/72785 | jq
            round: 72785,
            previous_signature: hex::decode("a609e19a03c2fcc559e8dae14900aaefe517cb55c840f6e69bc8e4f66c8d18e8a609685d9917efbfb0c37f058c2de88f13d297c7e19e0ab24813079efe57a182554ff054c7638153f9b26a60e7111f71a0ff63d9571704905d3ca6df0b031747").unwrap().into(),
            signature: hex::decode("82f5d3d2de4db19d40a6980e8aa37842a0e55d1df06bd68bddc8d60002e8e959eb9cfa368b3c1b77d18f02a54fe047b80f0989315f83b12a74fd8679c4f12aae86eaf6ab5690b34f1fddd50ee3cc6f6cdf59e95526d5a5d82aaa84fa6f181e42").unwrap().into(),
        };
        let result = execute(deps.as_mut(), mock_env(), info, msg);
        match result.unwrap_err() {
            ContractError::InvalidPubkey {} => {}
            err => panic!("Unexpected error: {:?}", err),
        }
    }

    #[test]
    fn add_fails_for_broken_signature() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        let info = mock_info("anyone", &[]);
        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/72785
            round: 72785,
            previous_signature: hex::decode("a609e19a03c2fcc559e8dae14900aaefe517cb55c840f6e69bc8e4f66c8d18e8a609685d9917efbfb0c37f058c2de88f13d297c7e19e0ab24813079efe57a182554ff054c7638153f9b26a60e7111f71a0ff63d9571704905d3ca6df0b031747").unwrap().into(),
            signature: hex::decode("3cc6f6cdf59e95526d5a5d82aaa84fa6f181e4").unwrap().into(), // broken signature
        };
        let result = execute(deps.as_mut(), mock_env(), info, msg);
        match result.unwrap_err() {
            ContractError::InvalidSignature {} => {}
            err => panic!("Unexpected error: {:?}", err),
        }
    }

    #[test]
    fn add_fails_for_invalid_signature() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        let info = mock_info("anyone", &[]);
        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/72785
            round: 1111, // wrong round
            previous_signature: hex::decode("a609e19a03c2fcc559e8dae14900aaefe517cb55c840f6e69bc8e4f66c8d18e8a609685d9917efbfb0c37f058c2de88f13d297c7e19e0ab24813079efe57a182554ff054c7638153f9b26a60e7111f71a0ff63d9571704905d3ca6df0b031747").unwrap().into(),
            signature: hex::decode("82f5d3d2de4db19d40a6980e8aa37842a0e55d1df06bd68bddc8d60002e8e959eb9cfa368b3c1b77d18f02a54fe047b80f0989315f83b12a74fd8679c4f12aae86eaf6ab5690b34f1fddd50ee3cc6f6cdf59e95526d5a5d82aaa84fa6f181e42").unwrap().into(),
        };
        let result = execute(deps.as_mut(), mock_env(), info, msg);
        match result.unwrap_err() {
            ContractError::InvalidSignature {} => {}
            err => panic!("Unexpected error: {:?}", err),
        }
    }

    #[test]
    fn query_get_works() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        // Beacon does not exist

        let response: GetResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Get { round: 42 }).unwrap())
                .unwrap();
        assert_eq!(response.randomness, Binary::default());

        // Beacon exists

        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/42 | jq
            round: 42,
            previous_signature: hex::decode("a418fccbfaa0c84aba8cbcd4e3c0555170eb2382dfed108ecfc6df249ad43efe00078bdcb5060fe2deed4731ca5b4c740069aaf77927ba59c5870ab3020352aca3853adfdb9162d40ec64f71b121285898e28cdf237e982ac5c4deb287b0d57b").unwrap().into(),
            signature: hex::decode("9469186f38e5acdac451940b1b22f737eb0de060b213f0326166c7882f2f82b92ce119bdabe385941ef46f72736a4b4d02ce206e1eb46cac53019caf870080fede024edcd1bd0225eb1335b83002ae1743393e83180e47d9948ab8ba7568dd99").unwrap().into(),
        };
        execute(deps.as_mut(), mock_env(), mock_info("anyone", &[]), msg).unwrap();

        let response: GetResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Get { round: 42 }).unwrap())
                .unwrap();
        assert_eq!(
            response.randomness,
            hex::decode("a9f12c5869d05e084d1741957130e1d0bf78a8ca9a8deb97c47cac29aae433c6")
                .unwrap()
        );
    }

    #[test]
    fn query_latest_fails_when_no_beacon_exists() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        let result = query(deps.as_ref(), mock_env(), QueryMsg::Latest {});
        match result.unwrap_err() {
            ContractError::NoBeacon {} => {}
            err => panic!("Unexpected error: {:?}", err),
        }
    }

    #[test]
    fn query_latest_returns_latest_beacon() {
        let mut deps = mock_dependencies();

        let info = mock_info("creator", &[]);
        let msg = InstantiateMsg {
            pubkey: pubkey_loe_mainnet(),
        };
        instantiate(deps.as_mut(), mock_env(), info, msg).unwrap();

        // Add first beacon

        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/42 | jq
            round: 42,
            previous_signature: hex::decode("a418fccbfaa0c84aba8cbcd4e3c0555170eb2382dfed108ecfc6df249ad43efe00078bdcb5060fe2deed4731ca5b4c740069aaf77927ba59c5870ab3020352aca3853adfdb9162d40ec64f71b121285898e28cdf237e982ac5c4deb287b0d57b").unwrap().into(),
            signature: hex::decode("9469186f38e5acdac451940b1b22f737eb0de060b213f0326166c7882f2f82b92ce119bdabe385941ef46f72736a4b4d02ce206e1eb46cac53019caf870080fede024edcd1bd0225eb1335b83002ae1743393e83180e47d9948ab8ba7568dd99").unwrap().into(),
        };
        execute(deps.as_mut(), mock_env(), mock_info("anyone", &[]), msg).unwrap();

        let latest: LatestResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Latest {}).unwrap()).unwrap();
        assert_eq!(latest.round, 42);
        assert_eq!(
            latest.randomness,
            hex::decode("a9f12c5869d05e084d1741957130e1d0bf78a8ca9a8deb97c47cac29aae433c6")
                .unwrap()
        );

        // Adding higher round updated the latest value

        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/45 | jq
            round: 45,
            previous_signature: hex::decode("a45dadaa23a0e70b06c297256c1bbdbcb915185c4bd2e0b6841e62f1b44264b82c8fc2ab97194e26ad90da55992d7c1e0cf0e58e17f91849aaecf545713b91efdebcb4cce06d3a0fcbabd72a8ab06050a3971898131e9026f29513680b99952a").unwrap().into(),
            signature: hex::decode("9280e40ac60dea6fcd936adbf69cae5c0add37fd161e036d34abd190099ddec975d15f9684d8875e4a69f5fe8ff9dde30fc29510fadde729a7d3b5522bbeddc4d2a08935025572daeee7d0130e55f51ff6d0dbbd15fc700151b420577072a801").unwrap().into(),
        };
        execute(deps.as_mut(), mock_env(), mock_info("anyone", &[]), msg).unwrap();

        let latest: LatestResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Latest {}).unwrap()).unwrap();
        assert_eq!(latest.round, 45);
        assert_eq!(
            latest.randomness,
            hex::decode("bfef28c6f445af5eedcf9de596a0bdd95b7e285aedefd17d70e1fac668c5f05b")
                .unwrap()
        );

        // Adding lower round does not affect latest

        let msg = ExecuteMsg::Add {
            // curl -sS https://drand.cloudflare.com/public/40 | jq
            round: 40,
            previous_signature: hex::decode("88756596758c8219b9973a496bf040a0962244c0a309695d92a9853ab03c1f5301ac9c02f8baeac6f84ce1a397f39eed1960be7f85b1c8bc64ac25567030a03673e08440d2a319319d883120a99822d0d6c23bd333725a1c4df269863a30b784").unwrap().into(),
            signature: hex::decode("8ea1d9cf15546a6b1515803dfaccbb379966b74e553fd9faa22206828e26d4b13a0b4d81f4820256af9bd228e428e2cb13a2bf634af151e815f939005b6393b12c33a7eed68d6c019ea3885f0a18541a23fb5312aab061d7ec9ebc798726a774").unwrap().into(),
        };
        execute(deps.as_mut(), mock_env(), mock_info("anyone", &[]), msg).unwrap();

        let latest: LatestResponse =
            from_binary(&query(deps.as_ref(), mock_env(), QueryMsg::Latest {}).unwrap()).unwrap();
        assert_eq!(latest.round, 45);
        assert_eq!(
            latest.randomness,
            hex::decode("bfef28c6f445af5eedcf9de596a0bdd95b7e285aedefd17d70e1fac668c5f05b")
                .unwrap()
        );
    }
}
