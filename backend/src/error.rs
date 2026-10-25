//! 带稳定错误码与参数的 API 响应，内部错误不泄漏文件或凭据内容。
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
/// 业务错误保留中文诊断用于服务日志，客户端按错误码本地化。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{message}")]
    Bad {
        code: &'static str,
        message: String,
        params: Value,
    },
    #[error("配置修订号已变化，请刷新后重试")]
    Conflict,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}
impl Error {
    /// 创建无参数的业务校验错误，错误码不随语言变化。
    pub fn bad(code: &'static str, message: impl Into<String>) -> Self {
        Self::with_params(code, message, json!({}))
    }
    /// 仅传递明确允许公开的插值参数；原始引擎诊断保留在 reason 中。
    pub fn with_params(code: &'static str, message: impl Into<String>, params: Value) -> Self {
        Self::Bad {
            code,
            message: message.into(),
            params,
        }
    }
    /// 公开错误结构也用于嵌套失败原因，不暴露内部 IO 和数据库详情。
    pub fn payload(&self) -> Value {
        match self {
            Self::Bad {
                code,
                message,
                params,
            } => json!({"code":code,"params":params,"error":message}),
            Self::Conflict => {
                json!({"code":"CONFIGURATION_CONFLICT","params":{},"error":self.to_string()})
            }
            _ => {
                json!({"code":"INTERNAL_ERROR","params":{},"error":"服务内部错误，请检查管理服务日志"})
            }
        }
    }
}
/// 套件内部结果别名。
pub type Result<T> = std::result::Result<T, Error>;
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::Bad { .. } => StatusCode::BAD_REQUEST,
            Self::Conflict => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        tracing::warn!(error=%self,"request failed");
        (status, Json(self.payload())).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 稳定错误码与参数不依赖展示语言，内部错误不公开底层路径。
    #[test]
    fn preserves_public_contract_and_hides_internal_details() {
        let error = Error::with_params("SITE_CODE_EXISTS", "代号已存在", json!({"code":"demo"}));
        let payload = error.payload();
        assert_eq!(payload["code"], "SITE_CODE_EXISTS");
        assert_eq!(payload["params"]["code"], "demo");
        assert_eq!(Error::Conflict.payload()["code"], "CONFIGURATION_CONFLICT");
        let internal = Error::Io(std::io::Error::other("private-path-and-secret"));
        assert_eq!(internal.payload()["code"], "INTERNAL_ERROR");
        assert!(
            !internal
                .payload()
                .to_string()
                .contains("private-path-and-secret")
        );
        let nested = Error::with_params(
            "SITE_CREATE_FAILED_FILES_KEPT",
            "创建失败",
            json!({"cause":error.payload(),"path":"/data/demo"}),
        );
        assert_eq!(
            nested.payload()["params"]["cause"]["code"],
            "SITE_CODE_EXISTS"
        );
    }
}
