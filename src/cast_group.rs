//! 把同一个媒体投到多个设备。
//!
//! 控制面是纯 UPnP 的：媒体数据由每个渲染器自己去 HTTP 服务器拉，所以
//! 「统一片源」就是给所有设备下发**同一个 URL**。
//!
//! [`CastGroup`](crate::cast_group::CastGroup) 把这个动作封装起来：命令并发下发给所有
//! 设备，一个设备失败不影响其它设备。

use crate::avtransport::{Cast, SeekUnit, TransportState};
use crate::ssdp::URN;
use crate::{Device, Error};
use futures_util::future::join_all;
use std::time::Duration;

/// 组里的一个设备。
#[derive(Debug, Clone)]
pub struct CastMember {
    /// 设备的友好名称，出错时用来定位是哪个设备。
    pub name: String,
    /// 该设备的投屏控制器。
    pub cast: Cast,
}

/// 单个设备的操作结果。
#[derive(Debug)]
pub struct MemberOutcome {
    /// 设备的友好名称。
    pub name: String,
    /// 操作结果。
    pub result: Result<(), Error>,
}

/// 一组设备执行操作后的结果。
///
/// 结果按设备在组里的顺序排列。
#[derive(Debug, Default)]
pub struct GroupOutcome {
    /// 每个设备的结果。
    pub members: Vec<MemberOutcome>,
}

impl GroupOutcome {
    /// 从「设备名 + 结果」的列表构造。
    fn from_pairs(results: Vec<(String, Result<(), Error>)>) -> Self {
        Self {
            members: results
                .into_iter()
                .map(|(name, result)| MemberOutcome { name, result })
                .collect(),
        }
    }

    /// 是不是所有设备都成功了。
    pub fn is_success(&self) -> bool {
        self.members.iter().all(|member| member.result.is_ok())
    }

    /// 成功的设备数。
    pub fn succeeded(&self) -> usize {
        self.members
            .iter()
            .filter(|member| member.result.is_ok())
            .count()
    }

    /// 失败的设备数。
    pub fn failed(&self) -> usize {
        self.members.len() - self.succeeded()
    }

    /// 失败的设备，依次给出名称和错误。
    pub fn failures(&self) -> impl Iterator<Item = (&str, &Error)> {
        self.members.iter().filter_map(|member| {
            member
                .result
                .as_ref()
                .err()
                .map(|err| (member.name.as_str(), err))
        })
    }

    /// 全部成功返回 `Ok(())`，否则把所有失败汇总成一个错误。
    pub fn into_result(self) -> Result<(), Error> {
        let failures: Vec<String> = self
            .members
            .iter()
            .filter_map(|member| {
                member
                    .result
                    .as_ref()
                    .err()
                    .map(|err| format!("{}: {err}", member.name))
            })
            .collect();

        if failures.is_empty() {
            Ok(())
        } else {
            Err(Error::StateNotReached(failures.join("；")))
        }
    }
}

/// 一组设备的投屏控制器。
///
/// 命令会**并发**下发给所有设备 —— 一个设备失败不影响其它设备继续收到命令。
///
/// # Example usage:
/// ```no_run
/// use rupnp::avtransport::TransportState;
/// use rupnp::cast_group::CastGroup;
/// use std::time::Duration;
///
/// # async fn cast_to_many(devices: Vec<rupnp::Device>) -> Result<(), rupnp::Error> {
/// let group = CastGroup::from_devices(&devices);
///
/// // 同一个 URL 下发给所有设备
/// let outcome = group
///     .set_uri_and_play("http://192.168.1.10:8080/movie.mp4", None)
///     .await;
///
/// // 一个设备失败不影响其它，逐个看结果
/// for (name, err) in outcome.failures() {
///     eprintln!("{name} 失败: {err}");
/// }
///
/// // 等所有设备都播起来
/// group
///     .wait_for_state(TransportState::Playing, Duration::from_secs(10))
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct CastGroup {
    members: Vec<CastMember>,
}

impl CastGroup {
    /// 创建一个空的组。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从设备列表里挑出带 AVTransport 服务的设备，建立投屏组。
    ///
    /// 没有 AVTransport 服务的设备会被跳过 —— 它们无法播放媒体。
    pub fn from_devices(devices: &[Device]) -> Self {
        let members = devices
            .iter()
            .filter_map(|device| {
                let service = device.find_service(&av_transport_urn())?;

                Some(CastMember {
                    name: device.friendly_name().to_string(),
                    cast: Cast::new(service.clone(), device.url().clone()),
                })
            })
            .collect();

        Self { members }
    }

    /// 从已有的 [`Cast`] 建立投屏组。
    pub fn from_casts(casts: impl IntoIterator<Item = Cast>) -> Self {
        let members = casts
            .into_iter()
            .enumerate()
            .map(|(index, cast)| CastMember {
                name: format!("设备 {index}"),
                cast,
            })
            .collect();

        Self { members }
    }

    /// 把一个设备加进组里。没有 AVTransport 服务时返回 `false`。
    pub fn add_device(&mut self, device: &Device) -> bool {
        let Some(service) = device.find_service(&av_transport_urn()) else {
            return false;
        };

        self.members.push(CastMember {
            name: device.friendly_name().to_string(),
            cast: Cast::new(service.clone(), device.url().clone()),
        });

        true
    }

    /// 组里的设备数。
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// 组是不是空的。
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// 组里的设备。
    pub fn members(&self) -> &[CastMember] {
        &self.members
    }

    /// 组里的投屏控制器。
    pub fn casts(&self) -> impl Iterator<Item = &Cast> {
        self.members.iter().map(|member| &member.cast)
    }

    /// 给所有设备下发同一个媒体地址。
    ///
    /// `metadata` 是 DIDL-Lite 元数据，所有设备收到的是同一份。
    pub async fn set_uri(&self, uri: &str, metadata: Option<&str>) -> GroupOutcome {
        let results = join_all(self.members.iter().map(|member| async move {
            (
                member.name.clone(),
                member.cast.set_uri(uri, metadata).await,
            )
        }))
        .await;

        GroupOutcome {
            members: results
                .into_iter()
                .map(|(name, result)| MemberOutcome { name, result })
                .collect(),
        }
    }

    /// 给所有设备下发媒体地址并开始播放。
    pub async fn set_uri_and_play(&self, uri: &str, metadata: Option<&str>) -> GroupOutcome {
        let results = join_all(self.members.iter().map(|member| async move {
            let outcome = member.cast.set_uri(uri, metadata).await;

            let outcome = match outcome {
                Ok(()) => member.cast.play().await,
                Err(err) => Err(err),
            };

            (member.name.clone(), outcome)
        }))
        .await;

        GroupOutcome {
            members: results
                .into_iter()
                .map(|(name, result)| MemberOutcome { name, result })
                .collect(),
        }
    }

    /// 让所有设备开始播放。
    pub async fn play(&self) -> GroupOutcome {
        let results = join_all(
            self.members
                .iter()
                .map(|member| async move { (member.name.clone(), member.cast.play().await) }),
        )
        .await;

        GroupOutcome::from_pairs(results)
    }

    /// 暂停所有设备。
    pub async fn pause(&self) -> GroupOutcome {
        let results = join_all(
            self.members
                .iter()
                .map(|member| async move { (member.name.clone(), member.cast.pause().await) }),
        )
        .await;

        GroupOutcome::from_pairs(results)
    }

    /// 停止所有设备。
    pub async fn stop(&self) -> GroupOutcome {
        let results = join_all(
            self.members
                .iter()
                .map(|member| async move { (member.name.clone(), member.cast.stop().await) }),
        )
        .await;

        GroupOutcome::from_pairs(results)
    }

    /// 让所有设备跳到指定位置。
    pub async fn seek(&self, unit: SeekUnit, target: &str) -> GroupOutcome {
        let results = join_all(self.members.iter().map(|member| async move {
            (member.name.clone(), member.cast.seek(unit, target).await)
        }))
        .await;

        GroupOutcome::from_pairs(results)
    }

    /// 查询所有设备当前的播放状态。
    pub async fn states(&self) -> Vec<Result<TransportState, Error>> {
        join_all(self.members.iter().map(|member| member.cast.state())).await
    }

    /// 等待所有设备都到达指定状态。
    ///
    /// 每个设备都有完整的 `timeout` 时间，所以总耗时不会超过它。
    /// 有设备没到达时返回 [`Error::StateNotReached`]，错误里会列出是哪几个。
    pub async fn wait_for_state(
        &self,
        state: TransportState,
        timeout: Duration,
    ) -> Result<(), Error> {
        let results = join_all(self.members.iter().map(|member| async move {
            (
                member.name.clone(),
                member.cast.wait_for_state(state, timeout).await,
            )
        }))
        .await;

        let failed: Vec<&str> = results
            .iter()
            .filter(|(_, result)| result.is_err())
            .map(|(name, _)| name.as_str())
            .collect();

        if failed.is_empty() {
            Ok(())
        } else {
            Err(Error::StateNotReached(format!(
                "{}（未到达的设备：{}）",
                state.as_str(),
                failed.join("、")
            )))
        }
    }
}

/// AVTransport 服务的 URN。
fn av_transport_urn() -> URN {
    URN::service("schemas-upnp-org", "AVTransport", 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(pairs: &[(&str, bool)]) -> GroupOutcome {
        GroupOutcome::from_pairs(
            pairs
                .iter()
                .map(|(name, ok)| {
                    (
                        name.to_string(),
                        if *ok {
                            Ok(())
                        } else {
                            Err(Error::StateNotReached("测试".to_string()))
                        },
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn empty_group_has_no_members() {
        let group = CastGroup::new();

        assert!(group.is_empty());
        assert_eq!(group.len(), 0);
    }

    #[test]
    fn av_transport_urn_is_correct() {
        assert_eq!(av_transport_urn().typ(), "AVTransport");
    }

    #[test]
    fn outcome_counts_successes_and_failures() {
        let outcome = outcome(&[("A", true), ("B", false), ("C", true)]);

        assert!(!outcome.is_success());
        assert_eq!(outcome.succeeded(), 2);
        assert_eq!(outcome.failed(), 1);
    }

    #[test]
    fn outcome_lists_failures_with_names() {
        let outcome = outcome(&[("A", true), ("B", false)]);

        let failures: Vec<&str> = outcome.failures().map(|(name, _)| name).collect();
        assert_eq!(failures, ["B"]);
    }

    #[test]
    fn outcome_converts_to_result() {
        // 全部成功
        assert!(outcome(&[("A", true), ("B", true)]).into_result().is_ok());

        // 有失败，错误信息里带上设备名
        let err = outcome(&[("A", true), ("电视", false)])
            .into_result()
            .unwrap_err();
        assert!(err.to_string().contains("电视"), "{err}");
    }
}
