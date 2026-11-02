use std::collections::HashMap;
use std::sync::Arc;

use zbus::object_server::ObjectServer;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::file_chooser::{Choice, choices, option};
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};

type Options = HashMap<String, OwnedValue>;
type Results = HashMap<String, OwnedValue>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Question {
    pub app_id: String,
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub deny_label: Option<String>,
    pub grant_label: Option<String>,
    pub icon: Option<String>,
    pub choices: Vec<Choice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Granted(Vec<(String, String)>),
    Denied,
    Unavailable,
}

pub trait AccessUi: Send + Sync {
    fn ask(&self, question: &Question, cancellation: &Cancellation) -> Answer;
}

pub struct Access {
    pub ui: Arc<dyn AccessUi>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Access")]
impl Access {
    #[allow(clippy::too_many_arguments)]
    async fn access_dialog(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        title: String,
        subtitle: String,
        body: String,
        options: Options,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, Results) {
        let question = Question {
            app_id,
            title,
            subtitle,
            body,
            deny_label: option(&options, "deny_label"),
            grant_label: option(&options, "grant_label"),
            icon: option(&options, "icon"),
            choices: choices(&options),
        };
        let asked = !question.choices.is_empty();
        let ui = Arc::clone(&self.ui);
        let answer = request::run(server, &handle, move |cancellation| {
            ui.ask(&question, cancellation)
        })
        .await;
        match answer {
            Ok(Some(Answer::Granted(chosen))) => {
                let mut results = Results::new();
                if asked && let Ok(chosen) = OwnedValue::try_from(Value::from(chosen)) {
                    results.insert("choices".to_owned(), chosen);
                }
                (SUCCESS, results)
            }
            Ok(Some(Answer::Denied) | None) => (CANCELLED, Results::new()),
            Ok(Some(Answer::Unavailable)) | Err(_) => (OTHER, Results::new()),
        }
    }
}
