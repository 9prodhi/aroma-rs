import { LightningElement, api, track, wire } from 'lwc';
export default class Sample extends LightningElement {
error(error) { this.dispatchEvent(new ShowToastEvent({ title: "Error", message: reduceErrors(error).join(",") })); }
}
