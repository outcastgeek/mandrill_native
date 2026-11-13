//! Native request context used by the canonical form service.
use std::sync::Arc;
use effect_server::Request;
use crate::validation::{ValidationArgument,ValidationTranslation};
pub type ValidationMessageTranslator=Arc<dyn Fn(&Request,&str,&[&ValidationArgument])->String+Send+Sync+'static>;
#[derive(Clone)]
pub struct CMSRequestContext{pub request:Request,pub validation_translate:Option<ValidationMessageTranslator>}
impl CMSRequestContext{pub fn new(request:&Request,validation_translate:Option<ValidationMessageTranslator>)->Self{Self{request:request.clone(),validation_translate}}}
impl ValidationTranslation for CMSRequestContext{fn translate(&self,key:&str,args:&[&ValidationArgument])->String{self.validation_translate.as_ref().map(|translate|translate(&self.request,key,args)).unwrap_or_else(||key.to_owned())}}
