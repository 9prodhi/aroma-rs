import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
select(event) { this.value = event?.detail?.value ?? ""; }
}
