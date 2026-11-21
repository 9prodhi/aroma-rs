import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
save(event) { this.value = event.detail.value; }
}
