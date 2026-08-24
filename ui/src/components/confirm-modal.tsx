import {
  ConfirmModal as MoghConfirmModal,
  ConfirmModalProps as MoghConfirmModalProps,
} from "mogh_ui";

export interface ConfirmModalProps extends Omit<
  MoghConfirmModalProps,
  "topAdditonal"
> {
  topAdditional?: MoghConfirmModalProps["topAdditonal"];
}

export function ConfirmModal({
  topAdditional,
  ...props
}: ConfirmModalProps) {
  return <MoghConfirmModal topAdditonal={topAdditional} {...props} />;
}
