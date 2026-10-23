import type { PairingPrompt } from "../lib/api";

interface Props {
  prompt: PairingPrompt;
  onAnswer: (accept: boolean) => void;
}

export function PairingDialog({ prompt, onAnswer }: Props) {
  const code = `${prompt.code.slice(0, 3)} ${prompt.code.slice(3)}`;
  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-labelledby="pairing-title">
        <h2 id="pairing-title">Pair with {prompt.device.name}?</h2>
        <p>Check that this code matches the one on the other device:</p>
        <p className="pairing-code">{code}</p>
        <p className="muted small">
          {prompt.device.os} · key {prompt.device.fingerprint}
        </p>
        <div className="row">
          <button onClick={() => onAnswer(false)}>Doesn't match</button>
          <button className="primary" onClick={() => onAnswer(true)} autoFocus>
            Codes match
          </button>
        </div>
      </div>
    </div>
  );
}
