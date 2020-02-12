use super::{ButtplugProtocol, ButtplugProtocolCreator};
use crate::{
    create_buttplug_protocol,
    device::{
        device::{
            ButtplugDeviceEvent, DeviceSubscribeCmd, DeviceUnsubscribeCmd,
        },
        configuration_manager::DeviceProtocolConfiguration
    },
};
use async_std::prelude::StreamExt;
use async_trait::async_trait;

pub struct LovenseProtocolCreator {
    config: DeviceProtocolConfiguration,
}

impl LovenseProtocolCreator {
    pub fn new(config: DeviceProtocolConfiguration) -> Self {
        Self { config }
    }
}

// TODO Figure out how we're gonna handle creation of protocols that require non-default initializers.
#[async_trait]
impl ButtplugProtocolCreator for LovenseProtocolCreator {
    async fn try_create_protocol(
        &self,
        device_impl: &Box<dyn DeviceImpl>,
    ) -> Result<Box<dyn ButtplugProtocol>, ButtplugError> {
        device_impl
        .subscribe(DeviceSubscribeCmd::new(Endpoint::Rx).into())
        .await?;
        let msg = DeviceWriteCmd::new(Endpoint::Tx, "DeviceType;".as_bytes().to_vec(), false);
        device_impl.write_value(msg.into()).await?;
        // TODO Put some sort of very quick timeout here, we should just fail if
        // we don't get something back quickly.
        let identifier;
        match device_impl.get_event_receiver().next().await {
            Some(ButtplugDeviceEvent::Notification(_, n)) => {
                let type_response = std::str::from_utf8(&n).unwrap().to_owned();
                info!("Lovense Device Type Response: {}", type_response);
                identifier = type_response.split(':').collect::<Vec<&str>>()[0].to_owned();
            }
            Some(ButtplugDeviceEvent::Removed) => {
                return Err(ButtplugDeviceError::new(
                    "Lovense Device disconnected while getting DeviceType info.",
                )
                .into());
            }
            None => {
                return Err(ButtplugDeviceError::new(
                    "Did not get DeviceType return from Lovense device in time",
                )
                .into());
            }
        };
        device_impl
        .unsubscribe(DeviceUnsubscribeCmd::new(Endpoint::Rx).into())
        .await?;

        let (names, attrs) = self.config.get_attributes(&identifier).unwrap();
        let name = names.get("en-us").unwrap();
        Ok(Box::new(Lovense::new(name, attrs)))
    }
}

create_buttplug_protocol!(
    Lovense,
    false,
    (
        (last_rotation: Arc<Mutex<Option<(u32, bool)>>> = Arc::new(Mutex::new(None)))
    ),
    ((VibrateCmd, {
        // Store off result before the match, so we drop the lock ASAP.
        let result = self.manager.lock().await.update_vibration(msg);
        // Lovense is the same situation as the Lovehoney Desire, where commands
        // are different if we're addressing all motors or seperate motors.
        // Difference here being that there's Lovense variants with different
        // numbers of motors.
        //
        // Neat way of checking if everything is the same via
        // https://sts10.github.io/2019/06/06/is-all-equal-function.html.
        //
        // Just make sure we're not matching on None, 'cause if that's the case
        // we ain't got shit to do.
        match result {
            Ok(cmds) => {
                if !cmds[0].is_none() && (cmds.len() == 1 || cmds.windows(2).all(|w| w[0] == w[1])) {
                    let lovense_cmd = format!("Vibrate:{};", cmds[0].unwrap()).as_bytes().to_vec();
                    device.write_value(DeviceWriteCmd::new(Endpoint::Tx, lovense_cmd, false)).await?;
                    return Ok(ButtplugMessageUnion::Ok(messages::Ok::default()));
                }
                for i in 0..cmds.len() {
                    if let Some(speed) = cmds[i] {
                        let lovense_cmd = format!("Vibrate{}:{};", i + 1, speed).as_bytes().to_vec();
                        device.write_value(DeviceWriteCmd::new(Endpoint::Tx, lovense_cmd, false)).await?;
                    }
                }
                return Ok(ButtplugMessageUnion::Ok(messages::Ok::default()));
            },
            Err(e) => Err(e)
        }
    }),
    (RotateCmd, {
        let result = self.manager.lock().await.update_rotation(msg);
        match result {
            Ok(cmds) => {
                // Due to lovense devices having separate commands for rotation
                // and speed, we can't completely depend on the generic command
                // manager here.
                //
                // TODO Should the generic command manager maybe store the
                // previous command as well as returning the next? That might
                // save us having to store this in the protocol members, but I'm
                // also not sure anyone but Lovense does this. For Vorze, we
                // need speed and direction regardless because they form a
                // single command.
                if let Some((speed, clockwise)) = cmds[0] {
                    let mut lovense_cmds = vec!();
                    {
                        let mut last_rotation = self.last_rotation.lock().await;
                        if let Some((rot_speed, rot_dir)) = *last_rotation {
                            if rot_dir != clockwise {
                                lovense_cmds.push("RotateChange;".as_bytes().to_vec());
                            }
                            if rot_speed != speed {
                                lovense_cmds.push(format!("Rotate:{};", speed).as_bytes().to_vec());
                            }
                        }
                        *last_rotation = Some((speed, clockwise));
                    }
                    for cmd in lovense_cmds {
                        device.write_value(DeviceWriteCmd::new(Endpoint::Tx, cmd, false)).await?;
                    }
                }
                Ok(ButtplugMessageUnion::Ok(messages::Ok::default()))
            },
            Err(e) => Err(e)
        }
    }))
);

// TODO Gonna need to add the ability to set subscribe data in tests before
// writing Lovense tests. Oops.
