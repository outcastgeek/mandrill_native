//! Inert typed native policy/component selections retained once by D18.
use crate::{
    IdentityError,
    capabilities::{Missing, Selected},
};
/// Typed optional native policy retrieval; missing selection is a typed expected failure.
pub trait PolicySlot {
    /// Actual selected service type; absent policy is uninhabited.
    type Service;
    /// Borrow the exact selected generation or return precise missing policy.
    fn selected_policy(&self, name: &'static str) -> Result<&Self::Service, IdentityError>;
}

/// Borrow the selected native token generation from the retained D18 policy bag.
pub trait TokenSelection {
    /// Actual application-selected native provider or registry composition.
    type Provider: Send + Sync;
    /// Missing selection remains an expected configuration result.
    fn token_provider(&self) -> Result<&Self::Provider, IdentityError>;
}
/// Borrow the selected native passkey relying-party policy.
pub trait PasskeySelection {
    /// Actual trusted RP/origin/verification selector.
    type Policy: Send + Sync;
    /// Never derive weaker settings from caller-supplied assurance fields.
    fn passkey_policy(&self) -> Result<&Self::Policy, IdentityError>;
}
/// Borrow the selected native email confirmation provider.
pub trait EmailSelection {
    /// Actual native purpose-bound provider.
    type Provider: Send + Sync;
    /// Preserve absence instead of instantiating a default token format.
    fn email_provider(&self) -> Result<&Self::Provider, IdentityError>;
}
/// Borrow the selected external protocol verifier.
pub trait ExternalSelection {
    /// Actual application-owned external evidence verifier.
    type Verifier: Send + Sync;
    /// Verification configuration is retained once by D18.
    fn external_verifier(&self) -> Result<&Self::Verifier, IdentityError>;
}
/// Borrow the selected server-backed browser context service.
pub trait BrowserSelection {
    /// Actual native origin/CSRF/pending context implementation.
    type Browser: Send + Sync;
    /// Preserve explicit browser-mode selection.
    fn browser_security(&self) -> Result<&Self::Browser, IdentityError>;
}
/// Borrow the selected native cookie/ticket service.
pub trait TicketSelection {
    /// Actual native session service, selected independently of stored mode.
    type Service: Send + Sync;
    /// Native services remain retained provider generations.
    fn ticket_service(&self) -> Result<&Self::Service, IdentityError>;
}
/// Borrow the selected native factor verifier for verified enrollment.
pub trait AuthenticatorSelection {
    /// Actual native provider over a current borrowed authenticator context.
    type Provider: Send + Sync;
    /// Missing factor selection never silently enables MFA.
    fn authenticator_provider(&self) -> Result<&Self::Provider, IdentityError>;
}
/// Borrow the selected native principal construction policy.
pub trait PrincipalSelection {
    /// Actual factory composition over scoped native current-user sources.
    type Principal: Send + Sync;
    /// Preserve the retained claim mapping and actual native sources.
    fn principal_policy(&self) -> Result<&Self::Principal, IdentityError>;
}
impl<Pw, At: PolicySlot, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr> AuthenticatorSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    At::Service: Send + Sync,
{
    type Provider = At::Service;
    /// Borrow the original selected native factor provider or preserve typed absence.
    fn authenticator_provider(&self) -> Result<&Self::Provider, IdentityError> {
        self.authenticator()
    }
}
impl<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr: PolicySlot, To, Ti, Kr> PrincipalSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    Pr::Service: Send + Sync,
{
    type Principal = Pr::Service;
    /// Borrow the retained native principal configuration or preserve typed absence.
    fn principal_policy(&self) -> Result<&Self::Principal, IdentityError> {
        self.principal_factory()
    }
}
impl<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To: PolicySlot, Ti, Kr> TokenSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    To::Service: Send + Sync,
{
    type Provider = To::Service;
    /// Borrow the retained native purpose-bound provider or preserve typed absence.
    fn token_provider(&self) -> Result<&Self::Provider, IdentityError> {
        self.tokens()
    }
}
impl<Pw, At, Ex, Pk: PolicySlot, Em, Br, Re, Va, No, Pr, To, Ti, Kr> PasskeySelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    Pk::Service: Send + Sync,
{
    type Policy = Pk::Service;
    /// Borrow the original trusted RP/origin/verification policy or preserve typed absence.
    fn passkey_policy(&self) -> Result<&Self::Policy, IdentityError> {
        self.passkeys()
    }
}
impl<Pw, At, Ex, Pk, Em: PolicySlot, Br, Re, Va, No, Pr, To, Ti, Kr> EmailSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    Em::Service: Send + Sync,
{
    type Provider = Em::Service;
    /// Borrow the retained native confirmation provider or preserve typed absence.
    fn email_provider(&self) -> Result<&Self::Provider, IdentityError> {
        self.email_confirmation()
    }
}
impl<Pw, At, Ex: PolicySlot, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr> ExternalSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    Ex::Service: Send + Sync,
{
    type Verifier = Ex::Service;
    /// Borrow the original trusted external evidence verifier or preserve typed absence.
    fn external_verifier(&self) -> Result<&Self::Verifier, IdentityError> {
        self.external()
    }
}
impl<Pw, At, Ex, Pk, Em, Br: PolicySlot, Re, Va, No, Pr, To, Ti, Kr> BrowserSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    Br::Service: Send + Sync,
{
    type Browser = Br::Service;
    /// Borrow selected server-backed browser configuration or preserve typed absence.
    fn browser_security(&self) -> Result<&Self::Browser, IdentityError> {
        self.browser()
    }
}
impl<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti: PolicySlot, Kr> TicketSelection
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
where
    Ti::Service: Send + Sync,
{
    type Service = Ti::Service;
    /// Borrow selected native cookie/ticket configuration or preserve typed absence.
    fn ticket_service(&self) -> Result<&Self::Service, IdentityError> {
        self.tickets()
    }
}
impl PolicySlot for Missing {
    type Service = std::convert::Infallible;
    /// Never manufacture a default policy/crypto/provider as fallback.
    fn selected_policy(&self, name: &'static str) -> Result<&Self::Service, IdentityError> {
        Err(IdentityError::MissingPolicy(name))
    }
}
impl<T> PolicySlot for Selected<T> {
    type Service = T;
    /// Borrow the actual selected service without reacquiring defaults.
    fn selected_policy(&self, _: &'static str) -> Result<&T, IdentityError> {
        Ok(self.service())
    }
}
/// Every replacement point remains typed, explicit and independently optional.
pub struct IdentityPolicies<
    Pw = Missing,
    At = Missing,
    Ex = Missing,
    Pk = Missing,
    Em = Missing,
    Br = Missing,
    Re = Missing,
    Va = Missing,
    No = Missing,
    Pr = Missing,
    To = Missing,
    Ti = Missing,
    Kr = Missing,
> {
    /// Retained explicit passwords selection.
    passwords: Pw,
    /// Retained explicit authenticator selection.
    authenticator: At,
    /// Retained explicit external selection.
    external: Ex,
    /// Retained explicit passkeys selection.
    passkeys: Pk,
    /// Retained explicit email confirmation selection.
    email_confirmation: Em,
    /// Retained explicit browser selection.
    browser: Br,
    /// Retained explicit renderers selection.
    renderers: Re,
    /// Retained explicit validators selection.
    validators: Va,
    /// Retained explicit normalizer selection.
    normalizer: No,
    /// Retained explicit principal factory selection.
    principal_factory: Pr,
    /// Retained explicit tokens selection.
    tokens: To,
    /// Retained explicit tickets selection.
    tickets: Ti,
    /// Retained explicit key ring selection.
    key_ring: Kr,
}
impl Default for IdentityPolicies {
    /// Preserve explicit absence of every policy without constructing any provider.
    fn default() -> Self {
        Self::new()
    }
}
/// Compile-time return types for replacing one policy slot; projection performs no provider work.
pub trait PolicySelections {
    /// Exact builder result after replacing Passwords; every other slot retains its type.
    type Passwords<Next>;
    /// Exact builder result after replacing Authenticator; every other slot retains its type.
    type Authenticator<Next>;
    /// Exact builder result after replacing External; every other slot retains its type.
    type External<Next>;
    /// Exact builder result after replacing Passkeys; every other slot retains its type.
    type Passkeys<Next>;
    /// Exact builder result after replacing EmailConfirmation; every other slot retains its type.
    type EmailConfirmation<Next>;
    /// Exact builder result after replacing Browser; every other slot retains its type.
    type Browser<Next>;
    /// Exact builder result after replacing Renderers; every other slot retains its type.
    type Renderers<Next>;
    /// Exact builder result after replacing Validators; every other slot retains its type.
    type Validators<Next>;
    /// Exact builder result after replacing Normalizer; every other slot retains its type.
    type Normalizer<Next>;
    /// Exact builder result after replacing PrincipalFactory; every other slot retains its type.
    type PrincipalFactory<Next>;
    /// Exact builder result after replacing Tokens; every other slot retains its type.
    type Tokens<Next>;
    /// Exact builder result after replacing Tickets; every other slot retains its type.
    type Tickets<Next>;
    /// Exact builder result after replacing KeyRing; every other slot retains its type.
    type KeyRing<Next>;
}
impl<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr> PolicySelections
    for IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
{
    /// Preserve the concrete bag and its existing slots when replacing Passwords.
    type Passwords<Next> =
        IdentityPolicies<Selected<Next>, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Authenticator.
    type Authenticator<Next> =
        IdentityPolicies<Pw, Selected<Next>, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing External.
    type External<Next> =
        IdentityPolicies<Pw, At, Selected<Next>, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Passkeys.
    type Passkeys<Next> =
        IdentityPolicies<Pw, At, Ex, Selected<Next>, Em, Br, Re, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing EmailConfirmation.
    type EmailConfirmation<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Selected<Next>, Br, Re, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Browser.
    type Browser<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Selected<Next>, Re, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Renderers.
    type Renderers<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Selected<Next>, Va, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Validators.
    type Validators<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Selected<Next>, No, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Normalizer.
    type Normalizer<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, Selected<Next>, Pr, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing PrincipalFactory.
    type PrincipalFactory<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Selected<Next>, To, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Tokens.
    type Tokens<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, Selected<Next>, Ti, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing Tickets.
    type Tickets<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Selected<Next>, Kr>;
    /// Preserve the concrete bag and its existing slots when replacing KeyRing.
    type KeyRing<Next> =
        IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Selected<Next>>;
}
/// Policy bag after retaining passwords; all other policy slots retain their exact types.
pub type WithPasswords<Current, Next> = <Current as PolicySelections>::Passwords<Next>;

/// Policy bag after retaining authenticator; all other policy slots retain their exact types.
pub type WithAuthenticator<Current, Next> = <Current as PolicySelections>::Authenticator<Next>;

/// Policy bag after retaining external; all other policy slots retain their exact types.
pub type WithExternal<Current, Next> = <Current as PolicySelections>::External<Next>;

/// Policy bag after retaining passkeys; all other policy slots retain their exact types.
pub type WithPasskeys<Current, Next> = <Current as PolicySelections>::Passkeys<Next>;

/// Policy bag after retaining email confirmation; all other policy slots retain their exact types.
pub type WithEmailConfirmation<Current, Next> =
    <Current as PolicySelections>::EmailConfirmation<Next>;

/// Policy bag after retaining browser; all other policy slots retain their exact types.
pub type WithBrowser<Current, Next> = <Current as PolicySelections>::Browser<Next>;

/// Policy bag after retaining renderers; all other policy slots retain their exact types.
pub type WithRenderers<Current, Next> = <Current as PolicySelections>::Renderers<Next>;

/// Policy bag after retaining validators; all other policy slots retain their exact types.
pub type WithValidators<Current, Next> = <Current as PolicySelections>::Validators<Next>;

/// Policy bag after retaining normalizer; all other policy slots retain their exact types.
pub type WithNormalizer<Current, Next> = <Current as PolicySelections>::Normalizer<Next>;

/// Policy bag after retaining principal factory; all other policy slots retain their exact types.
pub type WithPrincipalFactory<Current, Next> =
    <Current as PolicySelections>::PrincipalFactory<Next>;

/// Policy bag after retaining tokens; all other policy slots retain their exact types.
pub type WithTokens<Current, Next> = <Current as PolicySelections>::Tokens<Next>;

/// Policy bag after retaining tickets; all other policy slots retain their exact types.
pub type WithTickets<Current, Next> = <Current as PolicySelections>::Tickets<Next>;

/// Policy bag after retaining key ring; all other policy slots retain their exact types.
pub type WithKeyRing<Current, Next> = <Current as PolicySelections>::KeyRing<Next>;

impl IdentityPolicies {
    /// Start with explicit absence; selection never performs provider work.
    #[must_use]
    pub fn new() -> Self {
        Self {
            passwords: Missing,
            authenticator: Missing,
            external: Missing,
            passkeys: Missing,
            email_confirmation: Missing,
            browser: Missing,
            renderers: Missing,
            validators: Missing,
            normalizer: Missing,
            principal_factory: Missing,
            tokens: Missing,
            tickets: Missing,
            key_ring: Missing,
        }
    }
}
impl<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
    IdentityPolicies<Pw, At, Ex, Pk, Em, Br, Re, Va, No, Pr, To, Ti, Kr>
{
    /// Retain passwords once; native factories/workflows borrow this exact service.
    pub fn with_passwords<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithPasswords<Self, Next> {
        let Self {
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords: Selected::new(value),
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected passwords or a typed missing-selection result.
    pub fn passwords(&self) -> Result<&<Pw as PolicySlot>::Service, IdentityError>
    where
        Pw: PolicySlot,
    {
        self.passwords.selected_policy("passwords")
    }
    /// Retain authenticator once; native factories/workflows borrow this exact service.
    pub fn with_authenticator<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithAuthenticator<Self, Next> {
        let Self {
            passwords,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator: Selected::new(value),
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected authenticator or a typed missing-selection result.
    pub fn authenticator(&self) -> Result<&<At as PolicySlot>::Service, IdentityError>
    where
        At: PolicySlot,
    {
        self.authenticator.selected_policy("authenticator")
    }
    /// Retain external once; native factories/workflows borrow this exact service.
    pub fn with_external<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithExternal<Self, Next> {
        let Self {
            passwords,
            authenticator,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external: Selected::new(value),
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected external or a typed missing-selection result.
    pub fn external(&self) -> Result<&<Ex as PolicySlot>::Service, IdentityError>
    where
        Ex: PolicySlot,
    {
        self.external.selected_policy("external")
    }
    /// Retain passkeys once; native factories/workflows borrow this exact service.
    pub fn with_passkeys<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithPasskeys<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys: Selected::new(value),
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected passkeys or a typed missing-selection result.
    pub fn passkeys(&self) -> Result<&<Pk as PolicySlot>::Service, IdentityError>
    where
        Pk: PolicySlot,
    {
        self.passkeys.selected_policy("passkeys")
    }
    /// Retain email confirmation once; native factories/workflows borrow this exact service.
    pub fn with_email_confirmation<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithEmailConfirmation<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation: Selected::new(value),
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected email confirmation or a typed missing-selection result.
    pub fn email_confirmation(&self) -> Result<&<Em as PolicySlot>::Service, IdentityError>
    where
        Em: PolicySlot,
    {
        self.email_confirmation
            .selected_policy("email_confirmation")
    }
    /// Retain browser once; native factories/workflows borrow this exact service.
    pub fn with_browser<Next: Send + Sync + 'static>(self, value: Next) -> WithBrowser<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser: Selected::new(value),
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected browser or a typed missing-selection result.
    pub fn browser(&self) -> Result<&<Br as PolicySlot>::Service, IdentityError>
    where
        Br: PolicySlot,
    {
        self.browser.selected_policy("browser")
    }
    /// Retain renderers once; native factories/workflows borrow this exact service.
    pub fn with_renderers<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithRenderers<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers: Selected::new(value),
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected renderers or a typed missing-selection result.
    pub fn renderers(&self) -> Result<&<Re as PolicySlot>::Service, IdentityError>
    where
        Re: PolicySlot,
    {
        self.renderers.selected_policy("renderers")
    }
    /// Retain validators once; native factories/workflows borrow this exact service.
    pub fn with_validators<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithValidators<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators: Selected::new(value),
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected validators or a typed missing-selection result.
    pub fn validators(&self) -> Result<&<Va as PolicySlot>::Service, IdentityError>
    where
        Va: PolicySlot,
    {
        self.validators.selected_policy("validators")
    }
    /// Retain normalizer once; native factories/workflows borrow this exact service.
    pub fn with_normalizer<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithNormalizer<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            principal_factory,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer: Selected::new(value),
            principal_factory,
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected normalizer or a typed missing-selection result.
    pub fn normalizer(&self) -> Result<&<No as PolicySlot>::Service, IdentityError>
    where
        No: PolicySlot,
    {
        self.normalizer.selected_policy("normalizer")
    }
    /// Retain principal factory once; native factories/workflows borrow this exact service.
    pub fn with_principal_factory<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithPrincipalFactory<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            tokens,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory: Selected::new(value),
            tokens,
            tickets,
            key_ring,
        }
    }
    /// Borrow selected principal factory or a typed missing-selection result.
    pub fn principal_factory(&self) -> Result<&<Pr as PolicySlot>::Service, IdentityError>
    where
        Pr: PolicySlot,
    {
        self.principal_factory.selected_policy("principal_factory")
    }
    /// Retain tokens once; native factories/workflows borrow this exact service.
    pub fn with_tokens<Next: Send + Sync + 'static>(self, value: Next) -> WithTokens<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tickets,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens: Selected::new(value),
            tickets,
            key_ring,
        }
    }
    /// Borrow selected tokens or a typed missing-selection result.
    pub fn tokens(&self) -> Result<&<To as PolicySlot>::Service, IdentityError>
    where
        To: PolicySlot,
    {
        self.tokens.selected_policy("tokens")
    }
    /// Retain tickets once; native factories/workflows borrow this exact service.
    pub fn with_tickets<Next: Send + Sync + 'static>(self, value: Next) -> WithTickets<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            key_ring,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets: Selected::new(value),
            key_ring,
        }
    }
    /// Borrow selected tickets or a typed missing-selection result.
    pub fn tickets(&self) -> Result<&<Ti as PolicySlot>::Service, IdentityError>
    where
        Ti: PolicySlot,
    {
        self.tickets.selected_policy("tickets")
    }
    /// Retain key ring once; native factories/workflows borrow this exact service.
    pub fn with_key_ring<Next: Send + Sync + 'static>(
        self,
        value: Next,
    ) -> WithKeyRing<Self, Next> {
        let Self {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            ..
        } = self;
        IdentityPolicies {
            passwords,
            authenticator,
            external,
            passkeys,
            email_confirmation,
            browser,
            renderers,
            validators,
            normalizer,
            principal_factory,
            tokens,
            tickets,
            key_ring: Selected::new(value),
        }
    }
    /// Borrow selected key ring or a typed missing-selection result.
    pub fn key_ring(&self) -> Result<&<Kr as PolicySlot>::Service, IdentityError>
    where
        Kr: PolicySlot,
    {
        self.key_ring.selected_policy("key_ring")
    }
}
