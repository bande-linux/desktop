use oo7::{Key, crypto};
use zbus::zvariant::{Array, OwnedValue, Value};
use zeroize::Zeroizing;

pub const PLAIN: &str = "plain";
pub const DH_AES: &str = "dh-ietf1024-sha256-aes128-cbc-pkcs7";
const IV_LENGTH: usize = 16;
const GROUP_BYTES: usize = 128;

#[derive(Debug)]
pub enum Error {
    Unsupported,
    BadInput,
    Crypto,
}

pub struct Session {
    pub owner: String,
    aes: Option<Key>,
}

impl Session {
    pub fn open(
        owner: &str,
        algorithm: &str,
        input: &Value<'_>,
    ) -> Result<(Self, OwnedValue), Error> {
        match algorithm {
            PLAIN => Ok((
                Self {
                    owner: owner.to_owned(),
                    aes: None,
                },
                OwnedValue::from(zbus::zvariant::Str::from_static("")),
            )),
            DH_AES => {
                let client = input
                    .try_clone()
                    .ok()
                    .and_then(|value| Vec::<u8>::try_from(value).ok())
                    .ok_or(Error::BadInput)?;
                let significant = client.iter().skip_while(|byte| **byte == 0).count();
                if client.len() > GROUP_BYTES
                    || significant == 0
                    || client.ends_with(&[1]) && significant == 1
                {
                    return Err(Error::BadInput);
                }
                let private = Key::generate_private_key().map_err(|_| Error::Crypto)?;
                let public = Key::generate_public_key(&private).map_err(|_| Error::Crypto)?;
                let aes = Key::generate_aes_key(&private, &Key::new(client))
                    .map_err(|_| Error::Crypto)?;
                let output = Value::Array(Array::from(public.as_ref().to_vec()))
                    .try_into_owned()
                    .map_err(|_| Error::Crypto)?;
                Ok((
                    Self {
                        owner: owner.to_owned(),
                        aes: Some(aes),
                    },
                    output,
                ))
            }
            _ => Err(Error::Unsupported),
        }
    }

    pub fn encode(&self, secret: &[u8]) -> Result<(Vec<u8>, Vec<u8>), Error> {
        match &self.aes {
            None => Ok((Vec::new(), secret.to_vec())),
            Some(aes) => {
                let iv = crypto::generate_iv().map_err(|_| Error::Crypto)?;
                let value = crypto::encrypt(secret, aes, &iv).map_err(|_| Error::Crypto)?;
                Ok((iv, value))
            }
        }
    }

    pub fn decode(&self, parameters: &[u8], value: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error> {
        match &self.aes {
            None => Ok(Zeroizing::new(value.to_vec())),
            Some(_) if parameters.len() != IV_LENGTH => Err(Error::BadInput),
            Some(aes) => crypto::decrypt(value, aes, parameters)
                .map(|plain| Zeroizing::new(plain.to_vec()))
                .map_err(|_| Error::Crypto),
        }
    }
}
