import { onMount, type JSX } from "solid-js";

export function OwnerDialog(props: { class: string; title: string; close: () => void; children: JSX.Element }) {
  let dialog!: HTMLDialogElement;
  let onBackdrop = false;
  onMount(() => dialog.showModal());
  return <dialog ref={dialog} class={props.class} aria-labelledby={props.title}
    onCancel={(event) => { event.preventDefault(); props.close(); }}
    onPointerDown={(event) => { onBackdrop = event.target === event.currentTarget; }}
    onClick={(event) => { if (onBackdrop && event.target === event.currentTarget) props.close(); }}>
    {props.children}
  </dialog>;
}
