import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
run(value) { /* ignore */ this.value = value; // ignore
 return this.value; }
}
