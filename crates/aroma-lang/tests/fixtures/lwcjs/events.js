import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
notify(value) { this.dispatchEvent(new CustomEvent("change", { detail: value })); }
}
