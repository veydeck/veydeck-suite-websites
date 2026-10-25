//! Nginx HTTP 全局性能参数及边界校验。
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

/// 与已生效配置一起提交的全局参数；大小使用字段注明的单位。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Performance {
    pub revision: u64,
    pub server_names_hash_bucket_size: u32,
    pub client_header_buffer_size_kb: u32,
    pub client_max_body_size_mb: u32,
    pub keepalive_timeout: u32,
    pub gzip: bool,
    pub gzip_min_length: u32,
    pub gzip_comp_level: u8,
}
impl Default for Performance {
    /// 保留旧配置的 64 MiB 上传限制及官方镜像的其余默认行为。
    fn default() -> Self {
        Self {
            revision: 0,
            server_names_hash_bucket_size: 64,
            client_header_buffer_size_kb: 1,
            client_max_body_size_mb: 64,
            keepalive_timeout: 75,
            gzip: false,
            gzip_min_length: 20,
            gzip_comp_level: 1,
        }
    }
}
impl Performance {
    /// 限制数值范围，禁止通过参数注入原始 Nginx 指令。
    pub fn validate(&self) -> Result<()> {
        let bucket = self.server_names_hash_bucket_size;
        if !(32..=65536).contains(&bucket) || !bucket.is_power_of_two() {
            return Err(Error::bad(
                "HASH_BUCKET_SIZE_INVALID",
                "域名哈希桶大小必须为 32～65536 之间的 2 的幂",
            ));
        }
        if !(1..=1024).contains(&self.client_header_buffer_size_kb)
            || self.client_max_body_size_mb > 1_048_576
            || self.keepalive_timeout > 86400
            || self.gzip_min_length > 1_073_741_824
            || !(1..=9).contains(&self.gzip_comp_level)
        {
            return Err(Error::bad(
                "PERFORMANCE_OUT_OF_RANGE",
                "性能参数超出允许范围",
            ));
        }
        Ok(())
    }
    /// 生成 HTTP 上下文的指令，网站可显式覆盖允许继承的参数。
    pub fn directives(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "server_names_hash_bucket_size {};\nclient_header_buffer_size {}k;\nclient_max_body_size {}m;\nkeepalive_timeout {}s;\ngzip {};\ngzip_min_length {};\ngzip_comp_level {};\n",
            self.server_names_hash_bucket_size,
            self.client_header_buffer_size_kb,
            self.client_max_body_size_mb,
            self.keepalive_timeout,
            if self.gzip { "on" } else { "off" },
            self.gzip_min_length,
            self.gzip_comp_level
        ))
    }
}
