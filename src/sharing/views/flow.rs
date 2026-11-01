//! Presentation steps for the two file-based participant workflows.

use crate::{
    desktop::{i18n::Text, workspace::Unlocked},
    sharing::actions::{Artifact, Kind},
    vault::InvitationStatus,
};

/// A presentation snapshot keeps workflow decisions out of the rendered markup.
#[derive(Clone, Copy)]
pub(super) struct Workflow {
    step: Step,
    complete: bool,
    navigation_locked: bool,
    show_step_action: bool,
}

impl Workflow {
    pub fn new(unlocked: &Unlocked, selection: Option<Step>, now: u64) -> Self {
        let sharing = &unlocked.sharing;
        let complete = sharing.connected();
        // Reviewed and re-exported artifacts take precedence over manual navigation.
        let step = sharing
            .review
            .as_ref()
            .map(Step::reviewed)
            .or_else(|| {
                sharing
                    .output
                    .as_ref()
                    .map(|output| Step::output(output.kind))
            })
            .or(complete.then_some(Step::Connect))
            .or_else(|| {
                sharing
                    .saved_output
                    .as_ref()
                    .map(|output| match output.kind {
                        Kind::Certificate => Step::AcceptInvitation,
                        Kind::Invitation | Kind::Receipt => Step::ApproveAccess,
                        Kind::Response => Step::Connect,
                    })
            })
            .or(selection)
            .unwrap_or_else(|| Step::resume(unlocked, now));
        Self {
            step,
            complete,
            navigation_locked: sharing.has_artifact() || sharing.busy(),
            show_step_action: !complete && !sharing.has_artifact(),
        }
    }

    pub fn step(self) -> Step {
        self.step
    }

    pub fn title(self) -> Text {
        if self.complete {
            Text::SharingConnected
        } else {
            self.step.title()
        }
    }

    pub fn hint(self) -> Text {
        if self.complete {
            Text::SharingConnectedHint
        } else {
            self.step.hint()
        }
    }

    pub fn is_complete(self) -> bool {
        self.complete
    }

    pub fn navigation_locked(self) -> bool {
        self.navigation_locked
    }

    pub fn can_go_back(self) -> bool {
        !self.navigation_locked && self.step.previous().is_some()
    }

    pub fn show_step_action(self) -> bool {
        self.show_step_action
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Path {
    Invite,
    Join,
}

impl Path {
    pub fn steps(self) -> [Step; 3] {
        match self {
            Self::Invite => [Step::GetDevice, Step::SendInvitation, Step::ApproveAccess],
            Self::Join => [Step::PrepareDevice, Step::AcceptInvitation, Step::Connect],
        }
    }

    pub fn first(self) -> Step {
        self.steps()[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    GetDevice,
    SendInvitation,
    ApproveAccess,
    PrepareDevice,
    AcceptInvitation,
    Connect,
}

impl Step {
    pub fn reviewed(artifact: &Artifact) -> Self {
        match artifact {
            Artifact::Certificate(_) => Self::GetDevice,
            Artifact::Invitation(_) => Self::AcceptInvitation,
            Artifact::Response(_) => Self::ApproveAccess,
            Artifact::Receipt(_) => Self::Connect,
        }
    }

    pub fn output(kind: Kind) -> Self {
        match kind {
            Kind::Certificate => Self::PrepareDevice,
            Kind::Invitation => Self::SendInvitation,
            Kind::Response => Self::AcceptInvitation,
            Kind::Receipt => Self::ApproveAccess,
        }
    }

    fn resume(unlocked: &Unlocked, now: u64) -> Self {
        if unlocked
            .vault
            .accepted_invitations()
            .iter()
            .any(|invitation| !invitation.joined)
        {
            Self::Connect
        } else if unlocked
            .vault
            .issued_invitations(now)
            .iter()
            .any(|invitation| invitation.status == InvitationStatus::Pending)
        {
            Self::ApproveAccess
        } else if unlocked.sharing.joining {
            Self::PrepareDevice
        } else if unlocked.is_controller() {
            Self::GetDevice
        } else {
            Self::PrepareDevice
        }
    }

    pub fn path(self) -> Path {
        match self {
            Self::GetDevice | Self::SendInvitation | Self::ApproveAccess => Path::Invite,
            Self::PrepareDevice | Self::AcceptInvitation | Self::Connect => Path::Join,
        }
    }

    pub fn title(self) -> Text {
        match self {
            Self::GetDevice => Text::SharingStepDevice,
            Self::SendInvitation => Text::SharingStepInvite,
            Self::ApproveAccess => Text::SharingStepAdmit,
            Self::PrepareDevice => Text::SharingStepPrepare,
            Self::AcceptInvitation => Text::SharingStepAccept,
            Self::Connect => Text::SharingStepConnect,
        }
    }

    pub fn hint(self) -> Text {
        match self {
            Self::GetDevice => Text::SharingHintDevice,
            Self::SendInvitation => Text::SharingHintInvite,
            Self::ApproveAccess => Text::SharingHintAdmit,
            Self::PrepareDevice => Text::SharingHintPrepare,
            Self::AcceptInvitation => Text::SharingHintAccept,
            Self::Connect => Text::SharingHintConnect,
        }
    }

    pub fn previous(self) -> Option<Self> {
        match self {
            Self::GetDevice | Self::PrepareDevice => None,
            Self::SendInvitation => Some(Self::GetDevice),
            Self::ApproveAccess => Some(Self::SendInvitation),
            Self::AcceptInvitation => Some(Self::PrepareDevice),
            Self::Connect => Some(Self::AcceptInvitation),
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::GetDevice => Some(Self::SendInvitation),
            Self::SendInvitation => Some(Self::ApproveAccess),
            Self::PrepareDevice => Some(Self::AcceptInvitation),
            Self::AcceptInvitation => Some(Self::Connect),
            Self::ApproveAccess | Self::Connect => None,
        }
    }
}
