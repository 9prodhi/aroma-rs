import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
handle(event) { const { detail: { value }, target } = event; this.value = value; target.focus(); }
}
